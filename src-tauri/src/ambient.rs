//! Always-on (ambient) listening: dictate without touching the keyboard by
//! starting an utterance with the wake phrase ("Mouthpiece, ...").
//!
//! Pipeline per utterance: Silero VAD segments continuous audio → a
//! pre-roll is prepended so a soft first word isn't clipped → the segment
//! must match the enrolled voice → Whisper transcribes it → it must start
//! with the wake phrase → the rest goes through the normal dictation
//! pipeline, pasted only if the same app is still frontmost.
//!
//! The wake phrase is required on purpose: without it, a side conversation
//! or a phone call would eventually get typed into whatever was focused.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::AudioChunk;
use crate::capture::CaptureState;
use crate::recording::{process_transcript, RecordingState, TranscriptContext};

const SAMPLE_RATE: u32 = crate::stt::WHISPER_SAMPLE_RATE;
/// Silero's expected frame size at 16kHz.
const VAD_WINDOW: usize = 512;
/// Audio kept from before the VAD's detected start, so a soft onset that
/// Silero only recognizes a moment late still reaches Whisper.
const PREROLL: u64 = SAMPLE_RATE as u64;
const MIN_SILENCE_SECONDS: f32 = 0.7;
const MAX_SPEECH_SECONDS: f32 = 30.0;
/// Rolling history long enough for the longest segment plus its pre-roll.
const HISTORY_SAMPLES: usize = (SAMPLE_RATE as usize) * 33;
/// Falls asleep after this long with no wake-phrase use.
const AUTO_SLEEP_AFTER: Duration = Duration::from_secs(10 * 60);
/// How often to check whether another app (a call) has the microphone.
const MIC_CHECK_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbientPhase {
    Off,
    /// Wake phrase + command is dictated.
    Listening,
    /// Only "Mouthpiece, wake up" is acted on.
    Asleep,
    /// Another app (a call) is using the microphone; resumes on its own.
    Paused,
}

#[derive(Default)]
struct Metrics {
    segments: AtomicU64,
    speaker_rejects: AtomicU64,
    wake_hits: AtomicU64,
    wake_misses: AtomicU64,
}

impl Metrics {
    fn summary(&self) -> String {
        format!(
            "segments={} speaker_rejects={} wake_hits={} wake_misses={}",
            self.segments.load(Ordering::Relaxed),
            self.speaker_rejects.load(Ordering::Relaxed),
            self.wake_hits.load(Ordering::Relaxed),
            self.wake_misses.load(Ordering::Relaxed),
        )
    }
}

pub struct AmbientState {
    phase: Mutex<AmbientPhase>,
    stop: Mutex<Option<Arc<AtomicBool>>>,
    last_activity: Mutex<Instant>,
    metrics: Metrics,
}

impl AmbientState {
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(AmbientPhase::Off),
            stop: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            metrics: Metrics::default(),
        }
    }

    pub fn phase(&self) -> AmbientPhase {
        *self.phase.lock().unwrap()
    }
}

fn set_phase(app: &AppHandle, phase: AmbientPhase) {
    let state = app.state::<AmbientState>();
    let before = std::mem::replace(&mut *state.phase.lock().unwrap(), phase);
    if before != phase {
        crate::applog!("ambient: {before:?} -> {phase:?}");
        let _ = app.emit("ambient-phase", phase);
        crate::refresh_tray_indicator(app);
    }
}

fn touch(app: &AppHandle) {
    *app.state::<AmbientState>().last_activity.lock().unwrap() = Instant::now();
}

/// Starts listening. Refuses without an enrolled voice: the speaker check
/// is what keeps other people's speech out, and ambient mode without it
/// isn't safe to run.
pub fn start(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AmbientState>();
    let mut stop_slot = state.stop.lock().unwrap();
    if stop_slot.is_some() {
        return Ok(());
    }

    let vi = app.state::<crate::voice_isolation::VoiceIsolationState>();
    if vi.enrolled_embedding().is_none() {
        return Err("Enroll your voice first (Settings → Isolated Voice). Always-on listening only acts on your voice.".to_string());
    }

    let vad = create_vad(app)?;
    let (tx, rx) = mpsc::channel::<AudioChunk>();
    let stop = Arc::new(AtomicBool::new(false));
    *stop_slot = Some(stop.clone());
    drop(stop_slot);

    app.state::<RecordingState>().audio.set_tap(Some(tx));
    touch(app);
    set_phase(app, AmbientPhase::Listening);

    let worker_app = app.clone();
    std::thread::spawn(move || run_worker(&worker_app, vad, rx, &stop));
    Ok(())
}

pub fn stop(app: &AppHandle) {
    let state = app.state::<AmbientState>();
    let Some(stop) = state.stop.lock().unwrap().take() else {
        return;
    };
    stop.store(true, Ordering::SeqCst);
    app.state::<RecordingState>().audio.set_tap(None);
    crate::applog!("ambient: stopped ({})", state.metrics.summary());
    set_phase(app, AmbientPhase::Off);
}

pub(crate) fn vad_model_path(app: &AppHandle) -> Option<PathBuf> {
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("models").join("silero_vad.onnx"))
        .filter(|p| p.exists());
    bundled.or_else(|| {
        // `tauri dev` before a bundled build: the repo-local copy.
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("silero_vad.onnx");
        dev.exists().then_some(dev)
    })
}

fn create_vad(app: &AppHandle) -> Result<VoiceActivityDetector, String> {
    let model = vad_model_path(app).ok_or("speech detection model (silero_vad.onnx) not found")?;
    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(model.to_string_lossy().to_string()),
            threshold: 0.5,
            min_silence_duration: MIN_SILENCE_SECONDS,
            min_speech_duration: 0.25,
            window_size: VAD_WINDOW as i32,
            max_speech_duration: MAX_SPEECH_SECONDS,
        },
        sample_rate: SAMPLE_RATE as i32,
        num_threads: 1,
        ..Default::default()
    };
    VoiceActivityDetector::create(&config, MAX_SPEECH_SECONDS + 5.0)
        .ok_or_else(|| "couldn't initialize speech detection".to_string())
}

/// Recent audio indexed by absolute sample position (counted from the last
/// reset, the same origin the VAD uses for segment starts), so a segment
/// can be re-cut with pre-roll from before where the VAD placed its start.
struct History {
    buf: VecDeque<f32>,
    /// Absolute position of `buf[0]`.
    origin: u64,
    capacity: usize,
}

impl History {
    fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity),
            origin: 0,
            capacity,
        }
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.origin = 0;
    }

    fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples.iter().copied());
        let excess = self.buf.len().saturating_sub(self.capacity);
        if excess > 0 {
            self.buf.drain(..excess);
            self.origin += excess as u64;
        }
    }

    /// Samples in `[from, to)`, clamped to what's still retained.
    fn slice(&self, from: u64, to: u64) -> Vec<f32> {
        let end = self.origin + self.buf.len() as u64;
        let from = from.clamp(self.origin, end);
        let to = to.clamp(from, end);
        self.buf
            .range((from - self.origin) as usize..(to - self.origin) as usize)
            .copied()
            .collect()
    }
}

fn run_worker(
    app: &AppHandle,
    vad: VoiceActivityDetector,
    rx: mpsc::Receiver<AudioChunk>,
    stop: &AtomicBool,
) {
    crate::applog!("ambient: listening");
    let mut history = History::new(HISTORY_SAMPLES);
    let mut pending: Vec<f32> = Vec::new();
    let mut paused_for_capture = false;
    let mut last_mic_check = Instant::now() - MIC_CHECK_EVERY;
    let mut mic_check_unsupported_logged = false;

    while !stop.load(Ordering::SeqCst) {
        let chunk = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(chunk) => Some(chunk),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let state = app.state::<AmbientState>();
        if state.phase() == AmbientPhase::Listening
            && state.last_activity.lock().unwrap().elapsed() > AUTO_SLEEP_AFTER
        {
            crate::applog!("ambient: no wake-phrase use for {AUTO_SLEEP_AFTER:?}, going to sleep");
            set_phase(app, AmbientPhase::Asleep);
        }

        if last_mic_check.elapsed() >= MIC_CHECK_EVERY {
            last_mic_check = Instant::now();
            match (state.phase(), crate::mic_usage::other_processes_using_mic()) {
                (AmbientPhase::Listening, Some(pids)) if !pids.is_empty() => {
                    crate::applog!(
                        "ambient: another app is using the microphone (pids {pids:?}), pausing"
                    );
                    set_phase(app, AmbientPhase::Paused);
                }
                (AmbientPhase::Paused, Some(pids)) if pids.is_empty() => {
                    crate::applog!("ambient: microphone is free again, resuming");
                    touch(app);
                    set_phase(app, AmbientPhase::Listening);
                }
                (_, None) if !mic_check_unsupported_logged => {
                    mic_check_unsupported_logged = true;
                    crate::applog!("ambient: can't tell which apps use the mic on this macOS; no call auto-pause");
                }
                _ => {}
            }
        }

        let Some(chunk) = chunk else { continue };

        // Push-to-talk or enrollment owns the mic, or a call is going on:
        // ignore what we hear and start clean afterwards rather than
        // splicing across the gap.
        let busy = !matches!(
            app.state::<RecordingState>().capture.state(),
            CaptureState::Idle
        ) || state.phase() == AmbientPhase::Paused;
        if busy {
            if !paused_for_capture {
                vad.reset();
                history.reset();
                pending.clear();
                paused_for_capture = true;
            }
            continue;
        }
        paused_for_capture = false;

        let mono = crate::stt::downmix(&chunk.samples, chunk.channels);
        pending.extend(crate::stt::resample_linear(
            &mono,
            chunk.sample_rate,
            SAMPLE_RATE,
        ));
        while pending.len() >= VAD_WINDOW {
            let window: Vec<f32> = pending.drain(..VAD_WINDOW).collect();
            history.push(&window);
            vad.accept_waveform(&window);
        }

        while let Some(segment) = vad.front() {
            let start = segment.start().max(0) as u64;
            let end = start + segment.n().max(0) as u64;
            vad.pop();
            let audio = history.slice(start.saturating_sub(PREROLL), end);
            handle_segment(app, audio);
        }
    }
    crate::applog!("ambient: worker exited");
}

fn handle_segment(app: &AppHandle, audio: Vec<f32>) {
    let state = app.state::<AmbientState>();
    let metrics = &state.metrics;
    metrics.segments.fetch_add(1, Ordering::Relaxed);
    let seconds = audio.len() as f32 / SAMPLE_RATE as f32;

    // Fail closed: unlike push-to-talk (where you explicitly asked to
    // record), an ambient segment we can't attribute to you is dropped.
    match crate::isolate::enrolled_speaker_similarity(app, &audio) {
        Some(Ok(similarity)) if similarity >= crate::isolate::SIMILARITY_THRESHOLD => {
            crate::applog!("ambient: {seconds:.1}s segment, voice match {similarity:.2}");
        }
        other => {
            metrics.speaker_rejects.fetch_add(1, Ordering::Relaxed);
            let why = match other {
                Some(Ok(similarity)) => format!("voice match {similarity:.2} below threshold"),
                Some(Err(err)) => format!("couldn't score voice: {err}"),
                None => "no enrolled voice".to_string(),
            };
            crate::applog!(
                "ambient: {seconds:.1}s segment rejected ({why}); {}",
                metrics.summary()
            );
            return;
        }
    }

    let frontmost = crate::app_detect::frontmost_app_blocking(app);
    let recording = app.state::<RecordingState>();
    let raw = match recording.whisper.transcribe_samples(&audio, None) {
        Ok(text) => text,
        Err(err) => {
            crate::applog!("ambient: transcription failed: {err}");
            return;
        }
    };
    let text = crate::stt::strip_non_speech(&raw);
    crate::applog!("ambient: heard {}", crate::logging::said(&raw));

    match parse_wake(&text) {
        Addressed::Unaddressed => {
            metrics.wake_misses.fetch_add(1, Ordering::Relaxed);
            crate::applog!("ambient: no wake phrase, ignored; {}", metrics.summary());
        }
        Addressed::Sleep => {
            metrics.wake_hits.fetch_add(1, Ordering::Relaxed);
            set_phase(app, AmbientPhase::Asleep);
        }
        Addressed::Wake => {
            metrics.wake_hits.fetch_add(1, Ordering::Relaxed);
            touch(app);
            set_phase(app, AmbientPhase::Listening);
        }
        Addressed::Dictate(rest) => {
            metrics.wake_hits.fetch_add(1, Ordering::Relaxed);
            if state.phase() != AmbientPhase::Listening {
                crate::applog!("ambient: asleep, ignoring dictation (say \"Mouthpiece, wake up\")");
                return;
            }
            touch(app);
            crate::applog!("ambient: dictating; {}", metrics.summary());

            let cfg = crate::config::load(app);
            let bundle_id = frontmost.as_ref().map(|(id, _)| id.clone());
            let settings = crate::modes::resolve_settings(bundle_id.as_deref(), &cfg.modes);
            process_transcript(
                app,
                capitalize_first(rest),
                TranscriptContext {
                    cfg: &cfg,
                    settings: &settings,
                    app_name: frontmost.map(|(_, name)| name),
                    duration_ms: Some((seconds * 1000.0) as u64),
                    // Unknown frontmost app → an id that never matches, so
                    // the text is copied rather than pasted somewhere blind.
                    paste_guard: Some(bundle_id.unwrap_or_default()),
                    stopped_at: None,
                    stt_ms: None,
                    project: None,
                    reply_target: None,
                },
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Addressed<'a> {
    Dictate(&'a str),
    Sleep,
    Wake,
    Unaddressed,
}

/// Wake phrase spellings Whisper produces for "Mouthpiece" (tokens are
/// compared lowercased with punctuation stripped, so "Mouth piece." and
/// "mouth-piece" both match). The old name, "Dev Whisper", still works.
const WAKE_PHRASES: &[&[&str]] = &[
    &["mouthpiece"],
    &["mouth", "piece"],
    &["dev", "whisper"],
    &["devwhisper"],
    &["dev", "whisperer"],
];
const WAKE_PREFIXES: &[&str] = &["hey", "ok", "okay"];

fn normalize(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Only acts when the utterance *starts* with the wake phrase, so talking
/// about Mouthpiece mid-sentence doesn't trigger it.
fn parse_wake(text: &str) -> Addressed<'_> {
    let tokens: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w))
        .collect();
    let normalized: Vec<String> = tokens.iter().map(|(_, w)| normalize(w)).collect();

    let skip = usize::from(
        normalized
            .first()
            .is_some_and(|t| WAKE_PREFIXES.contains(&t.as_str())),
    );
    let Some(phrase_len) = WAKE_PHRASES.iter().find_map(|phrase| {
        let candidate = normalized.get(skip..skip + phrase.len())?;
        (candidate.iter().zip(phrase.iter()).all(|(a, b)| a == b)).then_some(phrase.len())
    }) else {
        return Addressed::Unaddressed;
    };

    let last = skip + phrase_len - 1;
    let (offset, word) = tokens[last];
    let rest = text[offset + word.len()..].trim_start_matches(|c: char| !c.is_alphanumeric());

    let command = normalized[last + 1..].join(" ");
    match command.as_str() {
        "" | "wake up" | "wake" => Addressed::Wake,
        "go to sleep" | "sleep" | "go to sleep now" => Addressed::Sleep,
        _ => Addressed::Dictate(rest),
    }
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[tauri::command]
pub fn get_ambient_enabled(app: AppHandle) -> bool {
    crate::config::load(&app).ambient_enabled
}

#[tauri::command]
pub fn set_ambient_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        start(&app)?;
    } else {
        stop(&app);
    }
    let mut cfg = crate::config::load(&app);
    cfg.ambient_enabled = enabled;
    crate::config::save(&app, &cfg)
}

#[tauri::command]
pub fn get_ambient_phase(state: tauri::State<AmbientState>) -> AmbientPhase {
    state.phase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictation_after_the_wake_phrase_keeps_original_text() {
        assert_eq!(
            parse_wake("Mouthpiece, send the report to Sam."),
            Addressed::Dictate("send the report to Sam.")
        );
    }

    #[test]
    fn wake_phrase_variants_match() {
        for text in [
            "Mouthpiece, ship it",
            "Hey mouth piece ship it",
            "mouth-piece: ship it",
            "dev-whisper ship it",
            "Hey Dev Whisper ship it",
            "Dev Whisperer, ship it",
            "OK, dev whisper: ship it",
        ] {
            assert_eq!(parse_wake(text), Addressed::Dictate("ship it"), "{text:?}");
        }
    }

    #[test]
    fn speech_without_the_wake_phrase_is_ignored() {
        assert_eq!(parse_wake("send the report to Sam"), Addressed::Unaddressed);
        assert_eq!(parse_wake(""), Addressed::Unaddressed);
    }

    #[test]
    fn mentioning_the_name_mid_sentence_is_not_addressing_it() {
        assert_eq!(
            parse_wake("I think Mouthpiece is great"),
            Addressed::Unaddressed
        );
    }

    #[test]
    fn sleep_and_wake_commands() {
        assert_eq!(parse_wake("Mouthpiece, go to sleep."), Addressed::Sleep);
        assert_eq!(parse_wake("Mouthpiece, wake up!"), Addressed::Wake);
        assert_eq!(parse_wake("Mouthpiece."), Addressed::Wake);
        assert_eq!(parse_wake("Dev Whisper, go to sleep."), Addressed::Sleep);
    }

    #[test]
    fn history_slices_by_absolute_position_with_preroll() {
        let mut h = History::new(10);
        h.push(&[0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(h.slice(1, 4), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn history_drops_oldest_and_clamps_requests_into_what_remains() {
        let mut h = History::new(4);
        h.push(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(h.origin, 2);
        assert_eq!(h.slice(0, 4), vec![2.0, 3.0]);
        assert_eq!(h.slice(4, 100), vec![4.0, 5.0]);
    }

    #[test]
    fn history_reset_restarts_positions_at_zero() {
        let mut h = History::new(4);
        h.push(&[1.0; 6]);
        h.reset();
        h.push(&[7.0, 8.0]);
        assert_eq!(h.slice(0, 2), vec![7.0, 8.0]);
    }

    #[test]
    fn capitalizes_only_the_first_letter() {
        assert_eq!(capitalize_first("send it to Sam"), "Send it to Sam");
        assert_eq!(capitalize_first(""), "");
    }

    /// Loads the real bundled Silero model through the same config the
    /// worker uses and checks silence yields no segment. (Synthetic tones
    /// don't reliably read as speech to Silero, so detection itself is
    /// covered by manual testing.) Skips if the model file isn't present.
    #[test]
    fn real_silero_model_loads_and_ignores_silence() {
        let model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("silero_vad.onnx");
        if !model.exists() {
            eprintln!("skipping: silero_vad.onnx not present");
            return;
        }
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model.to_string_lossy().to_string()),
                threshold: 0.5,
                min_silence_duration: MIN_SILENCE_SECONDS,
                min_speech_duration: 0.25,
                window_size: VAD_WINDOW as i32,
                max_speech_duration: MAX_SPEECH_SECONDS,
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: 1,
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&config, 35.0).expect("vad");

        // Pure silence must not produce a segment.
        for _ in 0..(SAMPLE_RATE as usize * 2 / VAD_WINDOW) {
            vad.accept_waveform(&[0.0; VAD_WINDOW]);
        }
        vad.flush();
        assert!(vad.front().is_none(), "silence produced a speech segment");
    }
}
