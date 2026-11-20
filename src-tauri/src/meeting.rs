//! Meeting mode: record a whole meeting through the mic, then write a
//! transcript split by speaker ("You", "Speaker 2", ...), all on this Mac.
//!
//! Recording streams to a 16 kHz WAV in the app data folder's `meetings/`
//! (flushed every few seconds, so a crash or quit still leaves a usable
//! file), from its own audio tap. While it runs the capture controller is
//! in `Meeting`, which keeps dictation, enrollment and hands-free listening
//! off the mic.
//!
//! After stopping: speaker diarization (sherpa-onnx: pyannote segmentation,
//! the bundled speaker-embedding model, then clustering) finds who spoke
//! when; neighbouring segments from one speaker are merged into turns;
//! each turn is transcribed with Whisper; and if a voice is enrolled, the
//! speaker that matches it is labelled "You". The transcript is saved as
//! `<id>.md` beside the recording, and the recording is deleted.

use crate::audio::AudioChunk;
use crate::capture::{Request, Transition};
use crate::stt::{downmix, resample_linear, WHISPER_SAMPLE_RATE};
use crate::RecordingState;
use serde::Serialize;
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const SEGMENTATION_MODEL: &str = "pyannote_segmentation_3_0.onnx";
/// Meetings stop themselves at 2.5 hours; the user starts a new one to keep
/// going. This bounds the disk (about 230 MB of stereo audio per hour) and,
/// more importantly, transcription memory: both channels are held in RAM as
/// f32 while diarizing and transcribing (about 1.15 GB at the cap).
const MAX_DURATION: Duration = Duration::from_secs(150 * 60);
/// How long before `MAX_DURATION` the widget warns that the meeting will stop.
const LIMIT_WARNING: Duration = Duration::from_secs(10 * 60);
/// Shorter than this and there's nothing worth separating.
const MIN_SECONDS: f32 = 2.0;
/// One speaker's segments closer together than this become one turn.
const MERGE_GAP_SECONDS: f32 = 1.5;
/// Shorter turns are mostly breaths and "mm", which Whisper turns into
/// made-up words.
const MIN_TURN_SECONDS: f32 = 0.5;
/// Whisper refuses clips under a second, so short turns are padded.
const MIN_CLIP_SECONDS: f32 = 1.2;
/// Enough audio per speaker to compare against the enrolled voice.
const YOU_SAMPLE_SECONDS: f32 = 30.0;
/// Turns are transcribed in pieces at most this long, so dictation never
/// waits behind one person's five-minute monologue for the Whisper lock.
const MAX_PIECE_SECONDS: f32 = 30.0;

/// A panic while transcribing mustn't wedge every later meeting.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
pub struct MeetingState {
    recording: Mutex<Option<Recording>>,
    progress: Mutex<Option<Progress>>,
    /// Meetings waiting to be transcribed or being transcribed, oldest
    /// first; they can't be deleted or queued twice.
    queue: Mutex<Vec<String>>,
    /// One transcript at a time: Whisper and the diarization models are
    /// heavy, and a second meeting can wait its turn.
    processing: Mutex<()>,
    /// Tracks consecutive call-detection polls and user dismissals so the
    /// widget can offer "Record <App>?" and "Call ended · Stop?".
    auto_detect: Mutex<crate::meeting_detect::AutoDetectTracker>,
}

struct Recording {
    id: String,
    started_ms: u64,
    writer: std::thread::JoinHandle<Result<(), String>>,
    system_tap: Option<crate::system_audio::SystemAudioTap>,
    speaker_poller: Option<crate::speaker_poller::SpeakerPollerHandle>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Progress {
    pub id: String,
    pub step: String,
    /// 0-100, or none while the step can't tell how far along it is.
    pub percent: Option<u8>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct MeetingStatus {
    /// Set while a meeting is being recorded: its id and start time.
    pub recording: Option<RecordingStatus>,
    pub processing: Option<Progress>,
    /// Every meeting waiting for or being transcribed.
    pub queued: Vec<String>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct RecordingStatus {
    pub id: String,
    pub started_ms: u64,
    /// The meeting stops itself this long after `started_ms`; the widget
    /// counts down to it once `LIMIT_WARNING` remains.
    pub limit_ms: u64,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MeetingFileState {
    Ready,
    /// The recording is there but has no transcript yet (processing
    /// failed, or the app quit before it finished).
    NotTranscribed,
    Recording,
    Processing,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    /// "32 min · 3 speakers", once transcribed.
    pub detail: Option<String>,
    pub state: MeetingFileState,
}

// ------------------------------------------------------------- recording

fn meetings_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("meetings");
    create_private_dir(&dir)?;
    Ok(dir)
}

fn create_private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// File name stem for a meeting started now, in local time: sorts by date
/// and reads well in Finder.
fn new_id(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y-%m-%d-%H%M%S").to_string()
}

/// When a meeting started, from its id (which may carry a "-2" suffix).
fn started_at(id: &str) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::NaiveDateTime::parse_from_str(id.get(..17)?, "%Y-%m-%d-%H%M%S")
        .ok()?
        .and_local_timezone(chrono::Local)
        .earliest()
}

/// Creates the recording file for a meeting starting `now`, picking an id
/// no other meeting has (two in one second, or the repeated hour when the
/// clocks go back, get a "-2" suffix).
fn create_recording(
    dir: &Path,
    now: chrono::DateTime<chrono::Local>,
) -> Result<(String, std::fs::File), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let base = new_id(now);
    for n in 1..100 {
        let id = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        if dir.join(format!("{id}.md")).exists() {
            continue;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dir.join(format!("{id}.wav")))
        {
            Ok(file) => return Ok((id, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("couldn't name the meeting recording".into())
}

/// Ids come back from the UI; only ever a plain file stem.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_digit() || c == '-')
}

fn checked_id(id: &str) -> Result<&str, String> {
    valid_id(id)
        .then_some(id)
        .ok_or_else(|| format!("not a meeting: {id:?}"))
}

/// Turns the mic's chunks (any rate, any channel count) into 16 kHz mono
/// in whole 100 ms blocks, so the linear resampler never sees a ragged
/// edge mid-recording.
#[derive(Default)]
struct To16k {
    rate: u32,
    pending: Vec<f32>,
}

impl To16k {
    fn push(&mut self, chunk: &AudioChunk, out: &mut Vec<f32>) {
        if chunk.sample_rate == 0 {
            return;
        }
        if chunk.sample_rate != self.rate {
            // The mic changed mid-meeting: finish the old rate first.
            self.finish(out);
            self.rate = chunk.sample_rate;
        }
        self.pending
            .extend(downmix(&chunk.samples, chunk.channels.max(1)));
        let block = (self.rate / 10).max(1) as usize;
        let whole = self.pending.len() / block * block;
        if whole > 0 {
            out.extend(resample_linear(
                &self.pending[..whole],
                self.rate,
                WHISPER_SAMPLE_RATE,
            ));
            self.pending.drain(..whole);
        }
    }

    /// Frames (at 16 kHz) buffered in the pending partial block.
    fn pending_16k(&self) -> usize {
        if self.rate == 0 {
            return 0;
        }
        (self.pending.len() as u64 * WHISPER_SAMPLE_RATE as u64 / self.rate as u64) as usize
    }

    fn finish(&mut self, out: &mut Vec<f32>) {
        if !self.pending.is_empty() && self.rate != 0 {
            out.extend(resample_linear(
                &self.pending,
                self.rate,
                WHISPER_SAMPLE_RATE,
            ));
        }
        self.pending.clear();
    }
}

/// Writes chunks to `file` until the audio thread drops the tap's sender.
fn write_meeting(file: std::fs::File, chunks: Receiver<AudioChunk>) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::new(std::io::BufWriter::new(file), spec).map_err(|e| e.to_string())?;
    let mut to16k = To16k::default();
    let mut block = Vec::new();
    let mut last_flush = Instant::now();
    let write = |samples: &mut Vec<f32>, writer: &mut hound::WavWriter<_>| -> Result<(), String> {
        for &s in samples.iter() {
            let value = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(value).map_err(|e| e.to_string())?;
        }
        samples.clear();
        Ok(())
    };
    for chunk in chunks {
        to16k.push(&chunk, &mut block);
        write(&mut block, &mut writer)?;
        if last_flush.elapsed() >= Duration::from_secs(5) {
            // Rewrites the header too, so the file is valid up to here.
            writer.flush().map_err(|e| e.to_string())?;
            last_flush = Instant::now();
        }
    }
    to16k.finish(&mut block);
    write(&mut block, &mut writer)?;
    writer.finalize().map_err(|e| e.to_string())
}

/// Writes a 2-channel 16 kHz WAV (`Left = Mic`, `Right = System`).
///
/// The mic stream is the master clock. Each stream's position is measured
/// in 16 kHz frames including `To16k`'s pending partial block, so 100 ms
/// block-boundary jitter never splices zeros into either channel.
///
/// Gaps are handled by aligning the two streams' *ends* (both chunks arrived
/// "now") whenever a stream resumes:
/// - System tap late to start, stalled, or restarted by its watchdog: mic
///   frames beyond `JITTER_TOLERANCE` are written against silence, and when
///   system audio resumes, the remaining unpaired mic backlog is also
///   written against silence, so the new system audio pairs with the mic
///   audio captured at the same moment. Without this, a 200 ms startup
///   offset persisted for the whole meeting and pushed real speakerphone
///   echo outside the echo gate's lag window.
/// - Mic stalled while the call keeps playing: system audio is written
///   against a silent mic instead of piling up unbounded in memory and then
///   being thrown away.
pub fn write_meeting_stereo(
    file: std::fs::File,
    mic_chunks: Receiver<AudioChunk>,
    sys_chunks: Receiver<AudioChunk>,
) -> Result<(), String> {
    use std::sync::mpsc::RecvTimeoutError;
    const JITTER_TOLERANCE: usize = (WHISPER_SAMPLE_RATE as usize) / 5; // 200 ms
    const MAX_SYS_LOOKAHEAD: usize = (WHISPER_SAMPLE_RATE as usize) / 2; // 500 ms
    const MIC_STALL: Duration = Duration::from_millis(300);
    const POLL: Duration = Duration::from_millis(100);

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::new(std::io::BufWriter::new(file), spec).map_err(|e| e.to_string())?;
    let mut mic_to16k = To16k::default();
    let mut sys_to16k = To16k::default();
    let mut mic_buf: Vec<f32> = Vec::new();
    let mut sys_buf: Vec<f32> = Vec::new();
    let mut last_flush = Instant::now();
    let mut last_mic = Instant::now();
    // The system stream hasn't started yet counts as stalled.
    let mut sys_stalled = true;
    let mut mic_stalled = false;

    let write_frames = |mic_slice: &[f32],
                        sys_slice: &[f32],
                        count: usize,
                        writer: &mut hound::WavWriter<_>|
     -> Result<(), String> {
        for i in 0..count {
            let m = mic_slice.get(i).copied().unwrap_or(0.0);
            let s = sys_slice.get(i).copied().unwrap_or(0.0);
            writer
                .write_sample((m.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|e| e.to_string())?;
            writer
                .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    };

    loop {
        let mut mic_arrived = false;
        match mic_chunks.recv_timeout(POLL) {
            Ok(chunk) => {
                mic_to16k.push(&chunk, &mut mic_buf);
                mic_arrived = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(more) = mic_chunks.try_recv() {
            mic_to16k.push(&more, &mut mic_buf);
            mic_arrived = true;
        }
        let mut sys_arrived = false;
        while let Ok(chunk) = sys_chunks.try_recv() {
            sys_to16k.push(&chunk, &mut sys_buf);
            sys_arrived = true;
        }
        if mic_arrived {
            last_mic = Instant::now();
        }
        let mic_end = mic_buf.len() + mic_to16k.pending_16k();
        let sys_end = sys_buf.len() + sys_to16k.pending_16k();

        // System resumed after a gap: write the mic backlog that predates it
        // against silence so both streams' ends line up.
        if sys_arrived && sys_stalled {
            let pad = mic_end.saturating_sub(sys_end).min(mic_buf.len());
            write_frames(&mic_buf[..pad], &[], pad, &mut writer)?;
            mic_buf.drain(..pad);
            sys_stalled = false;
        }
        // Mic resumed after a stall: same, the other way round.
        if mic_arrived && mic_stalled {
            let pad = sys_end.saturating_sub(mic_end).min(sys_buf.len());
            write_frames(&[], &sys_buf[..pad], pad, &mut writer)?;
            sys_buf.drain(..pad);
            mic_stalled = false;
        }

        // Pair everything available on both channels.
        let common = mic_buf.len().min(sys_buf.len());
        if common > 0 {
            write_frames(&mic_buf[..common], &sys_buf[..common], common, &mut writer)?;
            mic_buf.drain(..common);
            sys_buf.drain(..common);
        }

        if mic_buf.len() > JITTER_TOLERANCE {
            // System tap silent/stalled: keep the file advancing.
            let pad = mic_buf.len() - JITTER_TOLERANCE;
            write_frames(&mic_buf[..pad], &[], pad, &mut writer)?;
            mic_buf.drain(..pad);
            sys_stalled = true;
        }

        if last_mic.elapsed() >= MIC_STALL {
            // Mic stalled (device gone, profile switch) but the call goes on.
            if !sys_buf.is_empty() {
                let n = sys_buf.len();
                write_frames(&[], &sys_buf, n, &mut writer)?;
                sys_buf.clear();
            }
            mic_stalled = true;
        } else if sys_buf.len() > MAX_SYS_LOOKAHEAD {
            // Device clock skew: the mic is the master clock, so drop the
            // oldest excess system audio.
            let drop_count = sys_buf.len() - MAX_SYS_LOOKAHEAD;
            sys_buf.drain(..drop_count);
        }

        if last_flush.elapsed() >= Duration::from_secs(5) {
            // Rewrites the header too, so the file is valid up to here.
            writer.flush().map_err(|e| e.to_string())?;
            last_flush = Instant::now();
        }
    }

    while let Ok(chunk) = sys_chunks.try_recv() {
        sys_to16k.push(&chunk, &mut sys_buf);
    }
    mic_to16k.finish(&mut mic_buf);
    sys_to16k.finish(&mut sys_buf);
    let tail = mic_buf.len().max(sys_buf.len());
    if tail > 0 {
        write_frames(&mic_buf, &sys_buf, tail, &mut writer)?;
    }
    writer.finalize().map_err(|e| e.to_string())
}

pub fn status(app: &AppHandle) -> MeetingStatus {
    let state = app.state::<MeetingState>();
    let recording = lock(&state.recording).as_ref().map(|r| RecordingStatus {
        limit_ms: MAX_DURATION.as_millis() as u64,
        id: r.id.clone(),
        started_ms: r.started_ms,
    });
    let processing = lock(&state.progress).clone();
    let queued = lock(&state.queue).clone();
    MeetingStatus {
        recording,
        processing,
        queued,
    }
}

fn announce(app: &AppHandle) {
    let _ = app.emit("meeting-changed", status(app));
    crate::refresh_meeting_menu(app);
}

fn set_progress(app: &AppHandle, progress: Option<Progress>) {
    *lock(&app.state::<MeetingState>().progress) = progress;
    announce(app);
}

pub fn is_recording(app: &AppHandle) -> bool {
    app.try_state::<MeetingState>()
        .is_some_and(|s| lock(&s.recording).is_some())
}

/// Current RMS level of the system audio tap while a meeting is recording,
/// or 0.0 if no system tap is active.
pub fn current_system_audio_level(app: &AppHandle) -> f32 {
    app.try_state::<MeetingState>()
        .and_then(|s| {
            lock(&s.recording)
                .as_ref()
                .and_then(|r| r.system_tap.as_ref().map(|t| t.current_level()))
        })
        .unwrap_or(0.0)
}

/// Adds a meeting to the transcription queue; false if it's already there.
fn enqueue(app: &AppHandle, id: &str) -> bool {
    let state = app.state::<MeetingState>();
    let mut queue = lock(&state.queue);
    if queue.iter().any(|q| q == id) {
        return false;
    }
    queue.push(id.to_string());
    true
}

fn is_queued(app: &AppHandle, id: &str) -> bool {
    lock(&app.state::<MeetingState>().queue)
        .iter()
        .any(|q| q == id)
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    use crate::mic_permission::MicAccess;
    // Otherwise it would record hours of silence.
    let access = crate::mic_permission::status();
    if matches!(access, MicAccess::Denied | MicAccess::Restricted) {
        return Err(crate::mic_permission::silence_explanation(access)
            .unwrap_or("Mouthpiece isn't allowed to use the microphone.")
            .to_string());
    }
    let recording_state = app.state::<RecordingState>();
    let _op = lock(&recording_state.capture_op);
    let dir = meetings_dir(app)?;
    // The file exists before the meeting does, so a failure here leaves
    // nothing half-started.
    let (id, file) = create_recording(&dir, chrono::Local::now())?;
    let session = match recording_state.capture.request(Request::StartMeeting) {
        Transition::StartedMeeting { session } => session,
        other => {
            let _ = std::fs::remove_file(dir.join(format!("{id}.wav")));
            return Err(match other {
                Transition::Rejected(reason) => capitalized(reason),
                other => format!("unexpected {other:?}"),
            });
        }
    };

    let started_instant = Instant::now();
    let (mic_tx, mic_rx) = mpsc::channel();
    let (sys_tx, sys_rx) = mpsc::channel();

    // Attach the mic tap first so mic and system chunks begin accumulating at
    // the same instant.
    recording_state.audio.set_meeting_tap(Some(mic_tx));

    // Try starting the macOS 14.4+ CoreAudio Process Tap for dual-channel
    // capture (Left = Mic, Right = System). Fall back to mono mic-only if
    // unavailable or denied.
    let (system_tap, writer) = match crate::system_audio::SystemAudioTap::start(sys_tx) {
        Ok(tap) => {
            let writer = std::thread::spawn(move || write_meeting_stereo(file, mic_rx, sys_rx));
            (Some(tap), writer)
        }
        Err(reason) => {
            crate::applog!("meeting: system audio tap unavailable ({reason}); recording mic only");
            let writer = std::thread::spawn(move || write_meeting(file, mic_rx));
            (None, writer)
        }
    };

    let speaker_poller = Some(crate::speaker_poller::SpeakerPollerHandle::start(
        started_instant,
    ));

    *lock(&app.state::<MeetingState>().recording) = Some(Recording {
        id: id.clone(),
        started_ms: now_ms(),
        writer,
        system_tap,
        speaker_poller,
    });
    crate::applog!("meeting: recording {id}");
    crate::refresh_tray_indicator(app);
    crate::widget::show(app);
    let _ = app.emit("recording-started", "meeting");
    announce(app);

    // The widget's level meter, and the safety stop.
    #[derive(Clone, Copy, serde::Serialize)]
    struct DualAudioLevel {
        mic: f32,
        system: f32,
    }

    let level_app = app.clone();
    std::thread::spawn(move || {
        let state = level_app.state::<RecordingState>();
        let started = Instant::now();
        let mut warned = false;
        while state.capture.is_session_active(session) {
            // Push-to-talk during the meeting runs its own meter loop; two
            // loops emitting "audio-level" at once make the meter flicker.
            let dictating = matches!(
                state.capture.state(),
                crate::capture::CaptureState::Dictating { .. }
            );
            if !dictating {
                let mic_level = state.audio.current_level();
                let sys_level = current_system_audio_level(&level_app);
                let _ = level_app.emit(
                    "audio-level",
                    crate::audio::meter_level(mic_level.max(sys_level)),
                );
                let _ = level_app.emit(
                    "audio-dual-level",
                    DualAudioLevel {
                        mic: crate::audio::meter_level(mic_level),
                        system: crate::audio::meter_level(sys_level),
                    },
                );
                let _ = level_app.emit(
                    "audio-bands",
                    crate::recording::AudioBands::from_array(state.audio.current_bands()),
                );
            }
            if !warned && started.elapsed() >= MAX_DURATION - LIMIT_WARNING {
                warned = true;
                crate::applog!(
                    "meeting: {} minutes left before the limit",
                    LIMIT_WARNING.as_secs() / 60
                );
                // Shown even with "only while dictating" on: the user needs
                // to know the meeting is about to stop.
                crate::widget::show(&level_app);
                let _ = level_app.emit(
                    "meeting-limit-warning",
                    (LIMIT_WARNING.as_secs() / 60) as u32,
                );
            }
            if started.elapsed() >= MAX_DURATION {
                crate::applog!(
                    "meeting: stopping at the {}-minute limit",
                    MAX_DURATION.as_secs() / 60
                );
                let _ = stop(&level_app);
                crate::widget::show(&level_app);
                let _ = level_app.emit("meeting-limit-reached", ());
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    Ok(())
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn stop(app: &AppHandle) -> Result<(), String> {
    let recording_state = app.state::<RecordingState>();
    let recording = {
        let _op = lock(&recording_state.capture_op);
        match recording_state.capture.request(Request::StopMeeting) {
            Transition::StoppedMeeting { .. } => {}
            Transition::Rejected(reason) => return Err(capitalized(reason)),
            other => return Err(format!("unexpected {other:?}")),
        }
        // Dropping the tap's sender ends the writer thread.
        recording_state.audio.set_meeting_tap(None);
        lock(&app.state::<MeetingState>().recording).take()
    };
    crate::refresh_tray_indicator(app);
    let _ = app.emit("meeting-stopped", ());
    let Some(mut recording) = recording else {
        announce(app);
        return Ok(());
    };
    let id = recording.id.clone();
    let system_tap = recording.system_tap.take();
    let speaker_poller = recording.speaker_poller.take();

    enqueue(app, &id);
    if lock(&app.state::<MeetingState>().progress).is_none() {
        set_progress(
            app,
            Some(Progress {
                id: id.clone(),
                step: "Saving the recording".into(),
                percent: None,
            }),
        );
    } else {
        announce(app);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // Tearing down the CoreAudio tap joins its watchdog and can block;
        // keep it off the command thread.
        if let Some(tap) = system_tap {
            if !tap.has_non_zero_audio() {
                crate::applog!(
                    "meeting: system audio for {id} was silent the whole time \
                     (capture permission denied, or nothing played); transcribing mic only"
                );
            }
            drop(tap);
        }
        let observations = speaker_poller
            .map(|p| p.stop_and_take())
            .unwrap_or_default();
        if !observations.is_empty() {
            if let Ok(dir) = meetings_dir(&app) {
                if let Ok(json) = serde_json::to_string(&observations) {
                    let _ = write_private(&dir.join(format!("{id}.obs.json")), &json);
                }
            }
        }
        crate::applog!(
            "meeting: stopped {id} ({} speaker observations)",
            observations.len()
        );
        match recording.writer.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => crate::applog!("meeting: writing {id} failed: {err}"),
            Err(_) => crate::applog!("meeting: the writer for {id} panicked"),
        }
        transcribe_saved(&app, &id);
    });
    Ok(())
}

// ------------------------------------------------------------ processing

/// Takes a meeting off the queue (and its progress off the screen) when
/// transcribing it ends, however it ends, a panic included.
struct Dequeue<'a> {
    app: &'a AppHandle,
    id: &'a str,
}

impl Drop for Dequeue<'_> {
    fn drop(&mut self) {
        let state = self.app.state::<MeetingState>();
        lock(&state.queue).retain(|q| q != self.id);
        let mut progress = lock(&state.progress);
        if progress.as_ref().is_some_and(|p| p.id == self.id) {
            *progress = None;
        }
        drop(progress);
        announce(self.app);
    }
}

/// Turns a queued recording into its transcript, reporting progress, and
/// tells the widget how it went.
fn transcribe_saved(app: &AppHandle, id: &str) {
    let _dequeue = Dequeue { app, id };
    let state = app.state::<MeetingState>();
    let _one_at_a_time = lock(&state.processing);
    let started = Instant::now();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| transcribe_file(app, id)))
            .unwrap_or_else(|_| {
                Err(
            "Something went wrong while transcribing. The recording is kept; try Transcribe again."
                .into(),
        )
            });
    // Release the processing lock and dequeue before emitting `meeting-saved` /
    // `meeting-failed` so listeners calling `list_meetings` see `Ready` (not
    // `Processing`) and `meetingProcessing` is already cleared in the widget.
    drop(_one_at_a_time);
    drop(_dequeue);
    match result {
        Ok(summary) => {
            crate::applog!(
                "meeting: transcribed {id} in {:.0}s ({})",
                started.elapsed().as_secs_f32(),
                summary.detail.as_deref().unwrap_or("")
            );
            crate::widget::show(app);
            let _ = app.emit("meeting-saved", summary);
        }
        Err(err) => {
            crate::applog!("meeting: couldn't transcribe {id}: {err}");
            crate::widget::show(app);
            let _ = app.emit("meeting-failed", err);
        }
    }
}

const YOU_SPEAKER_ID: i32 = -1;

fn load_observations(dir: &Path, id: &str) -> Vec<crate::speaker_poller::SpeakerObservation> {
    let path = dir.join(format!("{id}.obs.json"));
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn transcribe_file(app: &AppHandle, id: &str) -> Result<MeetingSummary, String> {
    let dir = meetings_dir(app)?;
    let wav = dir.join(format!("{id}.wav"));
    let obs_path = dir.join(format!("{id}.obs.json"));
    let progress = |step: &str, percent: Option<u8>| {
        set_progress(
            app,
            Some(Progress {
                id: id.to_string(),
                step: step.to_string(),
                percent,
            }),
        )
    };
    progress("Finding who spoke", None);
    let LoadedRecording {
        mut mic,
        system: sys_opt,
    } = load_recording_channels(&wav)?;
    let duration = mic.len() as f32 / WHISPER_SAMPLE_RATE as f32;
    if duration < MIN_SECONDS {
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_file(&obs_path);
        return Err("The meeting was too short to transcribe.".into());
    }

    let speakers = crate::config::load(app).meeting_speakers;
    let models = DiarizationModels {
        segmentation: resource_path(app, SEGMENTATION_MODEL)?,
        embedding: crate::voice_isolation::resolve_model_path(app)?,
    };
    let observations = load_observations(&dir, id);

    let has_active_system = sys_opt.as_ref().is_some_and(|sys| has_call_audio(sys));

    let (turns, names) = if let Some(sys_samples) = sys_opt.as_ref().filter(|_| has_active_system) {
        // Dual-channel path:
        // 1. Suppress speakerphone acoustic bleed from `system` into `mic`.
        suppress_speakerphone_echo(&mut mic, sys_samples);

        // 2. Segment the local microphone track directly as "You".
        let you_segments = if crate::audio::is_dead_air(&mic) {
            Vec::new()
        } else {
            segment_single_speaker_segments(&mic, &models, YOU_SPEAKER_ID)?
        };

        // 3. Diarize the remote system audio track, merge the two channels
        //    into turns (see `merge_dual_channel_turns`), and fuse UI names.
        //    Since "You" is already isolated on Channel 0, pass `speakers - 1`
        //    as the remote speaker count when a total speaker count is configured.
        let remote_speakers = if speakers > 0 {
            (speakers - 1).max(1)
        } else {
            0
        };
        let remote = diarize_segments_with_observations(
            sys_samples,
            &models,
            remote_speakers,
            &observations,
        )?;
        let all_turns = merge_dual_channel_turns(&you_segments, &remote.segments);
        let mut names = resolve_cluster_names(&all_turns, &observations, Some(YOU_SPEAKER_ID));
        apply_remote_group_fallback(
            &mut names,
            &all_turns,
            &observations,
            speakers,
            remote.confirmed_single_voice,
        );
        names.insert(YOU_SPEAKER_ID, "You".to_string());
        let remote_clusters = all_turns
            .iter()
            .filter(|t| t.speaker != YOU_SPEAKER_ID)
            .map(|t| t.speaker)
            .collect::<std::collections::HashSet<_>>()
            .len();
        crate::applog!(
            "meeting: diarized {id} (stereo, you_segs={}, remote_segs={}, remote_clusters={remote_clusters}, single_voice={}, ui_obs={})",
            you_segments.len(),
            remote.segments.len(),
            remote.confirmed_single_voice,
            observations.len()
        );
        (all_turns, names)
    } else {
        // Single-channel fallback (mono WAV or silent system channel during an
        // in-person room meeting).
        let outcome = diarize_segments_with_observations(&mic, &models, speakers, &observations)?;
        let turns = merge_turns(&outcome.segments);
        let you = find_you(app, &mic, &turns);
        let names = resolve_cluster_names(&turns, &observations, you);
        let cluster_count = turns
            .iter()
            .map(|t| t.speaker)
            .collect::<std::collections::HashSet<_>>()
            .len();
        crate::applog!(
            "meeting: diarized {id} (mono, segs={}, clusters={cluster_count}, ui_obs={})",
            outcome.segments.len(),
            observations.len()
        );
        (turns, names)
    };

    if turns.is_empty() {
        let sys_dead = sys_opt
            .as_ref()
            .is_none_or(|s| crate::audio::is_dead_air(s));
        if crate::audio::is_dead_air(&mic) && sys_dead {
            let _ = std::fs::remove_file(&wav);
            let _ = std::fs::remove_file(&obs_path);
            return Err("The meeting recording was silent. Check the microphone.".into());
        }
        return Err("No speech was found in the meeting recording.".into());
    }

    let whisper = &app.state::<RecordingState>().whisper;
    let mut lines = Vec::new();
    let total = turns.len();
    for (index, turn) in turns.iter().enumerate() {
        progress("Transcribing", Some((index * 100 / total).min(99) as u8));
        let source_samples = match (&sys_opt, has_active_system, turn.speaker == YOU_SPEAKER_ID) {
            (Some(sys_samples), true, false) => sys_samples.as_slice(),
            _ => mic.as_slice(),
        };
        let clip = clip(source_samples, turn);
        let mut text = String::new();
        for piece in pieces(&clip) {
            let context = format!("{} {text}", recent_context(&lines));
            let heard =
                whisper.transcribe_samples_in_context(piece, None, context.trim(), &|_| {})?;
            let heard = crate::stt::strip_non_speech(&heard);
            if !heard.trim().is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(heard.trim());
            }
        }
        if !text.is_empty() {
            lines.push(Line {
                start: turn.start,
                speaker: names
                    .get(&turn.speaker)
                    .cloned()
                    .unwrap_or_else(|| "Speaker 1".to_string()),
                text,
            });
        }
    }
    if lines.is_empty() {
        return Err("No speech was found in the meeting recording.".into());
    }

    progress("Saving", Some(100));
    let title = meeting_title(started_at(id).unwrap_or_else(chrono::Local::now));
    let detail = meeting_detail(duration, &lines);
    let markdown = render_markdown(&title, &detail, &lines);
    write_private(&dir.join(format!("{id}.md")), &markdown)?;
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&obs_path);
    Ok(MeetingSummary {
        id: id.to_string(),
        title,
        detail: Some(detail),
        state: MeetingFileState::Ready,
    })
}

/// Whether the system channel carries a call's worth of audio, rather than
/// dead air (denied capture permission, in-room meeting) or a stray chime or
/// short clip. Only then is the mic channel treated as "You" alone; otherwise
/// the mic is diarized so several people in the room stay distinct.
fn has_call_audio(system: &[f32]) -> bool {
    const FRAME: usize = 320; // 20 ms
    const ACTIVE_RMS: f32 = 0.01;
    const MAX_REQUIRED_SECS: f32 = 10.0;
    const MIN_FRACTION: f32 = 0.2;
    let secs = system.len() as f32 / WHISPER_SAMPLE_RATE as f32;
    let required = MAX_REQUIRED_SECS.min(secs * MIN_FRACTION);
    let active_frames = system
        .chunks_exact(FRAME)
        .filter(|f| (f.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt() >= ACTIVE_RMS)
        .count();
    active_frames as f32 * FRAME as f32 / WHISPER_SAMPLE_RATE as f32 >= required
}

/// De-interleaved recording channels loaded from `<id>.wav`.
pub struct LoadedRecording {
    pub mic: Vec<f32>,
    pub system: Option<Vec<f32>>,
}

/// Loads either a 2-channel 16 kHz stereo meeting recording (`Left = mic`,
/// `Right = system`) or a 1-channel mono recording.
pub fn load_recording_channels(path: &Path) -> Result<LoadedRecording, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels == 2
        && spec.sample_rate == WHISPER_SAMPLE_RATE
        && spec.bits_per_sample == 16
        && spec.sample_format == hound::SampleFormat::Int
    {
        let frames = (reader.len() as usize) / 2;
        let mut mic = Vec::with_capacity(frames);
        let mut system = Vec::with_capacity(frames);
        let mut samples = reader.samples::<i16>().map_while(Result::ok);
        while let (Some(left), Some(right)) = (samples.next(), samples.next()) {
            mic.push(left as f32 / i16::MAX as f32);
            system.push(right as f32 / i16::MAX as f32);
        }
        return Ok(LoadedRecording {
            mic,
            system: Some(system),
        });
    }
    Ok(LoadedRecording {
        mic: load_recording(path)?,
        system: None,
    })
}

/// The recording as samples. It's always 16 kHz mono 16-bit, read straight
/// into one buffer (a 4-hour meeting is ~900 MB as f32; the general loader
/// would hold three copies). A crash can leave a partial last sample,
/// which is skipped.
#[allow(dead_code)]
fn load_recording(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != WHISPER_SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return crate::stt::load_samples_16k_mono(path);
    }
    let mut samples = Vec::with_capacity(reader.len() as usize);
    samples.extend(
        reader
            .samples::<i16>()
            .map_while(Result::ok)
            .map(|s| s as f32 / i16::MAX as f32),
    );
    Ok(samples)
}

/// Conference-Room Echo Gate: detects and zeroes speakerphone bleed in `mic`
/// where remote speech from `system` played out of the Mac's speakers and
/// re-entered the microphone with an acoustic lag between `0ms` and `120ms`.
///
/// Preserves double-talk when the local user speaks louder than the remote
/// bleed (`mic_rms > 1.25 * sys_rms`).
pub fn suppress_speakerphone_echo(mic: &mut [f32], system: &[f32]) {
    const FRAME_SAMPLES: usize = 320; // 20ms at 16 kHz
    const MAX_LAG_SAMPLES: usize = 1920; // 120ms at 16 kHz
    const LAG_STEP_SAMPLES: usize = 16; // 1ms coarse step
    const MIN_SYS_RMS: f32 = 0.015;
    const ECHO_NCC_THRESHOLD: f32 = 0.55;
    const DOUBLE_TALK_RATIO: f32 = 1.25;
    const ATTACK_WALKBACK_FRAMES: usize = 4; // 80ms
    const HANGOVER_FRAMES: usize = 7; // 140ms

    let len = mic.len().min(system.len());
    let num_frames = len / FRAME_SAMPLES;
    if num_frames == 0 {
        return;
    }

    // Prefix sums of squared system samples: any window's energy in O(1)
    // instead of 320 multiply-adds at each of ~150 lags per frame.
    let mut sys_sq = Vec::with_capacity(len + 1);
    sys_sq.push(0.0f64);
    let mut acc = 0.0f64;
    for &x in &system[..len] {
        acc += (x as f64) * (x as f64);
        sys_sq.push(acc);
    }
    let energy = |start: usize, n: usize| (sys_sq[start + n] - sys_sq[start]).max(0.0) as f32;
    let min_sys_energy = MIN_SYS_RMS * MIN_SYS_RMS * FRAME_SAMPLES as f32;

    let mut echo_frames = vec![false; num_frames];

    for (frame_idx, is_echo) in echo_frames.iter_mut().enumerate() {
        let pos = frame_idx * FRAME_SAMPLES;
        let mic_frame = &mic[pos..pos + FRAME_SAMPLES];
        let mic_energy: f32 = mic_frame.iter().map(|&x| x * x).sum();
        let mic_rms = (mic_energy / FRAME_SAMPLES as f32).sqrt();
        if mic_rms < 0.001 {
            continue;
        }

        let max_lag = pos.min(MAX_LAG_SAMPLES);
        // No window in the lag span can reach MIN_SYS_RMS if the whole span
        // is quieter than one such window: nothing to correlate against.
        if energy(pos - max_lag, max_lag + FRAME_SAMPLES) < min_sys_energy {
            continue;
        }
        let mut best_ncc = 0.0f32;
        let mut best_lag = 0usize;
        let mut best_sys_rms = 0.0f32;

        let mut lag = 0usize;
        while lag <= max_lag {
            let sys_start = pos - lag;
            let sys_frame = &system[sys_start..sys_start + FRAME_SAMPLES];
            let sys_energy = energy(sys_start, FRAME_SAMPLES);
            let sys_rms = (sys_energy / FRAME_SAMPLES as f32).sqrt();
            if sys_rms >= MIN_SYS_RMS {
                let dot: f32 = mic_frame
                    .iter()
                    .zip(sys_frame.iter())
                    .map(|(&m, &s)| m * s)
                    .sum();
                let denom = (mic_energy * sys_energy).sqrt();
                if denom > 1e-9 {
                    let ncc = (dot / denom).abs();
                    if ncc > best_ncc {
                        best_ncc = ncc;
                        best_lag = lag;
                        best_sys_rms = sys_rms;
                    }
                }
            }
            lag += LAG_STEP_SAMPLES;
        }

        // Fine 1-sample search around `best_lag` so high-frequency waveforms
        // between coarse lag steps still hit their exact phase peak.
        if best_ncc > 0.0 {
            let fine_start = best_lag.saturating_sub(LAG_STEP_SAMPLES);
            let fine_end = (best_lag + LAG_STEP_SAMPLES).min(max_lag);
            for fine_lag in fine_start..=fine_end {
                let sys_start = pos - fine_lag;
                let sys_frame = &system[sys_start..sys_start + FRAME_SAMPLES];
                let sys_energy = energy(sys_start, FRAME_SAMPLES);
                let sys_rms = (sys_energy / FRAME_SAMPLES as f32).sqrt();
                if sys_rms >= MIN_SYS_RMS {
                    let dot: f32 = mic_frame
                        .iter()
                        .zip(sys_frame.iter())
                        .map(|(&m, &s)| m * s)
                        .sum();
                    let denom = (mic_energy * sys_energy).sqrt();
                    if denom > 1e-9 {
                        let ncc = (dot / denom).abs();
                        if ncc > best_ncc {
                            best_ncc = ncc;
                            best_sys_rms = sys_rms;
                        }
                    }
                }
            }
        }

        if best_ncc >= ECHO_NCC_THRESHOLD && mic_rms <= DOUBLE_TALK_RATIO * best_sys_rms {
            *is_echo = true;
        }
    }

    // Apply attack walkback and hangover to cover room reverb onset/tails.
    let mut masked = echo_frames.clone();
    for (i, _) in echo_frames.iter().enumerate().filter(|(_, &e)| e) {
        let start = i.saturating_sub(ATTACK_WALKBACK_FRAMES);
        let end = (i + HANGOVER_FRAMES).min(num_frames - 1);
        masked[start..=end].fill(true);
    }

    for (frame_idx, &is_echo) in masked.iter().enumerate() {
        if is_echo {
            let pos = frame_idx * FRAME_SAMPLES;
            mic[pos..pos + FRAME_SAMPLES].fill(0.0);
        }
    }
}

/// Finds speech segments on a single-speaker track (the local microphone
/// channel in stereo meeting mode) and labels all segments with `speaker_id`.
fn segment_single_speaker_segments(
    samples: &[f32],
    models: &DiarizationModels,
    speaker_id: i32,
) -> Result<Vec<(f32, f32, i32)>, String> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2);
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(models.segmentation.to_string_lossy().to_string()),
            },
            num_threads: threads,
            ..Default::default()
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(models.embedding.to_string_lossy().to_string()),
            num_threads: threads,
            ..Default::default()
        },
        clustering: FastClusteringConfig {
            num_clusters: 1,
            threshold: 0.9,
        },
        ..Default::default()
    };
    let diarizer = OfflineSpeakerDiarization::create(&config)
        .ok_or("couldn't load the speaker separation models")?;
    let result = diarizer
        .process(samples)
        .ok_or("speaker separation failed")?;
    let labelled: Vec<(f32, f32, i32)> = result
        .sort_by_start_time()
        .into_iter()
        .filter(|s| s.end - s.start >= MIN_SEGMENT_SECONDS)
        .map(|s| (s.start, s.end, speaker_id))
        .collect();
    Ok(labelled)
}

/// A clip cut into roughly equal pieces of at most `MAX_PIECE_SECONDS`.
fn pieces(clip: &[f32]) -> std::slice::Chunks<'_, f32> {
    let max = (MAX_PIECE_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
    let count = clip.len().div_ceil(max).max(1);
    clip.chunks(clip.len().div_ceil(count).max(1))
}

/// Writes via a temp file in the same folder and renames it over `path`,
/// so a crash or full disk mid-write never leaves a half-written
/// transcript: renames and notes overwrite the only copy.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("not a file path")?;
    // A counter too, so a rename and a summary saving at once don't share
    // one temp file.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.map_err(|e| e.to_string())
}

fn resource_path(app: &AppHandle, file: &str) -> Result<PathBuf, String> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("models").join(file))
        .filter(|p| p.exists())
        .or_else(|| {
            // `tauri dev` before a bundled build: the repo-local copy.
            let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join(file);
            dev.exists().then_some(dev)
        })
        .ok_or_else(|| format!("speaker separation model ({file}) not found"))
}

pub struct DiarizationModels {
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
}

/// Who spoke when, as turns. `speakers` is how many people there were, or
/// 0 to work it out.
///
/// sherpa-onnx finds the speech segments, but its own speaker grouping
/// works on short windows and blurs similar voices together, even when
/// told the speaker count. So it's run to over-split (its grouping only
/// shapes the segment boundaries), long segments with mid-segment speaker
/// handoffs are split at local pause valleys, and `group_voices` decides
/// who is who.
#[cfg(test)]
pub fn diarize(
    samples: &[f32],
    models: &DiarizationModels,
    speakers: u32,
) -> Result<Vec<Turn>, String> {
    let outcome = diarize_segments_with_observations(samples, models, speakers, &[])?;
    Ok(merge_turns(&outcome.segments))
}

struct DiarizationOutcome {
    segments: Vec<(f32, f32, i32)>,
    /// When all speech landed in a single cluster, whether acoustic evidence
    /// positively confirms one consistent voice rather than short/noisy or
    /// borderline-spread clips that couldn't be separated.
    confirmed_single_voice: bool,
}

fn diarize_segments_with_observations(
    samples: &[f32],
    models: &DiarizationModels,
    speakers: u32,
    observations: &[crate::speaker_poller::SpeakerObservation],
) -> Result<DiarizationOutcome, String> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2);
    let embedding = SpeakerEmbeddingExtractorConfig {
        model: Some(models.embedding.to_string_lossy().to_string()),
        num_threads: threads,
        ..Default::default()
    };
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(models.segmentation.to_string_lossy().to_string()),
            },
            num_threads: threads,
            ..Default::default()
        },
        embedding: embedding.clone(),
        clustering: FastClusteringConfig {
            num_clusters: -1,
            threshold: OVERSPLIT_THRESHOLD,
        },
        ..Default::default()
    };
    let diarizer = OfflineSpeakerDiarization::create(&config)
        .ok_or("couldn't load the speaker separation models")?;
    let result = diarizer
        .process(samples)
        .ok_or("speaker separation failed")?;
    let raw_segments: Vec<(f32, f32)> = result
        .sort_by_start_time()
        .into_iter()
        .map(|s| (s.start, s.end))
        .filter(|(start, end)| end - start >= MIN_SEGMENT_SECONDS)
        .collect();
    drop(diarizer);

    let spans = observation_spans(observations);
    let ui_split = if spans.is_empty() {
        raw_segments
    } else {
        split_segments_at_ui_changes(samples, &raw_segments, &spans)
    };

    let extractor = SpeakerEmbeddingExtractor::create(&embedding)
        .ok_or("couldn't load the speaker embedding model")?;
    let segments = split_segments_at_voice_changes(samples, ui_split, &extractor);

    let voices: Vec<Voice> = segments
        .iter()
        .map(|&(start, end)| Voice {
            embedding: extract_embedding(&extractor, samples, start, end),
            seconds: end - start,
        })
        .collect();
    let ui_hints: Vec<Option<String>> = if spans.is_empty() {
        Vec::new()
    } else {
        segments
            .iter()
            .map(|&(start, end)| dominant_ui_speaker(start, end, &spans).map(str::to_string))
            .collect()
    };
    let hints_ref = (!ui_hints.is_empty()).then_some(ui_hints.as_slice());
    let labels = group_voices_with_hints(&voices, hints_ref, speakers, SAME_SPEAKER_SIMILARITY);

    let unique_clusters: std::collections::HashSet<usize> = labels.iter().copied().collect();
    let single_half_similarity = if unique_clusters.len() <= 1 {
        let anchors: Vec<usize> = (0..voices.len())
            .filter(|&i| voices[i].embedding.is_some() && voices[i].seconds >= ANCHOR_SECONDS)
            .collect();
        if anchors.len() == 1 {
            let (start, end) = segments[anchors[0]];
            if end - start >= 3.2 {
                let mid = (start + end) * 0.5;
                let split_t = quietest_split_time(samples, mid, start + 1.4, end - 1.4, 0.35);
                match (
                    extract_embedding(&extractor, samples, start, split_t),
                    extract_embedding(&extractor, samples, split_t, end),
                ) {
                    (Some(h1), Some(h2)) => Some(cosine(&h1, &h2)),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let confirmed_single_voice = unique_clusters.len() == 1
        && single_voice_consistent_from_embeddings(&voices, single_half_similarity);

    let labelled: Vec<(f32, f32, i32)> = segments
        .iter()
        .zip(labels)
        .map(|(&(start, end), label)| (start, end, label as i32))
        .collect();
    Ok(DiarizationOutcome {
        segments: labelled,
        confirmed_single_voice,
    })
}

fn extract_embedding(
    extractor: &SpeakerEmbeddingExtractor,
    samples: &[f32],
    start: f32,
    end: f32,
) -> Option<Vec<f32>> {
    let rate = WHISPER_SAMPLE_RATE as f32;
    let a = ((start * rate) as usize).min(samples.len());
    let b = ((end * rate) as usize).clamp(a, samples.len());
    if b <= a {
        return None;
    }
    let stream = extractor.create_stream()?;
    stream.accept_waveform(WHISPER_SAMPLE_RATE as i32, &samples[a..b]);
    stream.input_finished();
    extractor.is_ready(&stream).then_some(())?;
    // A NaN or infinite value poisons every similarity it touches (NaN
    // never compares below the threshold), so treat it as no voice print.
    extractor
        .compute(&stream)
        .filter(|v| v.iter().all(|x| x.is_finite()))
}

/// Finds the center timestamp (in seconds) of the quietest 20ms frame near
/// `target_sec`, clamped to `[min_sec, max_sec]`, so segment splits land in
/// natural syllable/breath pauses rather than mid-word.
fn quietest_split_time(
    samples: &[f32],
    target_sec: f32,
    min_sec: f32,
    max_sec: f32,
    search_radius_sec: f32,
) -> f32 {
    const WINDOW: usize = 320; // 20ms at 16 kHz
    const STEP: usize = 80; // 5ms step
    if min_sec >= max_sec {
        return target_sec.clamp(min_sec.min(max_sec), min_sec.max(max_sec));
    }
    let rate = WHISPER_SAMPLE_RATE as f32;
    let lo_sec = (target_sec - search_radius_sec).max(min_sec);
    let hi_sec = (target_sec + search_radius_sec).min(max_sec);
    let lo = ((lo_sec * rate) as usize).min(samples.len());
    let hi = ((hi_sec * rate) as usize).clamp(lo, samples.len());
    if hi < lo + WINDOW {
        return target_sec.clamp(min_sec, max_sec);
    }
    let mut best_sample = ((target_sec.clamp(min_sec, max_sec) * rate) as usize).min(samples.len());
    let mut min_energy = f32::INFINITY;
    let mut pos = lo;
    while pos + WINDOW <= hi {
        let energy: f32 = samples[pos..pos + WINDOW].iter().map(|x| x * x).sum();
        if energy < min_energy {
            min_energy = energy;
            best_sample = pos + WINDOW / 2;
        }
        pos += STEP;
    }
    (best_sample as f32 / rate).clamp(min_sec, max_sec)
}

/// Splits segments where Accessibility UI tile observations show an
/// unambiguous active-speaker handoff from one participant to another.
fn split_segments_at_ui_changes(
    samples: &[f32],
    segments: &[(f32, f32)],
    spans: &[(f32, f32, &crate::speaker_poller::SpeakerObservation)],
) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(segments.len());
    for &(seg_start, seg_end) in segments {
        let mut cur = seg_start;
        while seg_end - cur >= 2.2 {
            let mut split_at: Option<f32> = None;
            for &(obs_start, _, _) in spans {
                if obs_start <= cur + 0.9 || obs_start >= seg_end - 0.9 {
                    continue;
                }
                let before = dominant_ui_speaker(cur.max(obs_start - 2.5), obs_start, spans);
                let after = dominant_ui_speaker(obs_start, seg_end.min(obs_start + 2.5), spans);
                if let (Some(b), Some(a)) = (before, after) {
                    if a != b {
                        split_at = Some(quietest_split_time(
                            samples,
                            obs_start,
                            cur + 0.8,
                            seg_end - 0.8,
                            0.35,
                        ));
                        break;
                    }
                }
            }
            if let Some(t) = split_at {
                out.push((cur, t));
                cur = t;
            } else {
                break;
            }
        }
        out.push((cur, seg_end));
    }
    out
}

/// Checks long speech segments (`>= MIN_SPLIT_SEGMENT_SECONDS`) for missed
/// back-to-back speaker handoffs (common in rapid stand-up updates) and splits
/// at the quietest pause valley when the two sides' voice embeddings diverge.
fn split_segments_at_voice_changes(
    samples: &[f32],
    segments: Vec<(f32, f32)>,
    extractor: &SpeakerEmbeddingExtractor,
) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(segments.len());
    let mut queue: std::collections::VecDeque<(f32, f32, u8)> =
        segments.into_iter().map(|(s, e)| (s, e, 0)).collect();
    while let Some((start, end, depth)) = queue.pop_front() {
        let dur = end - start;
        if dur >= MIN_SPLIT_SEGMENT_SECONDS && depth < 3 {
            let fractions: &[f32] = if dur >= 10.0 {
                &[0.33, 0.5, 0.67]
            } else {
                &[0.5]
            };
            // Under ~2 s a half's voice print is too noisy to trust: a pause
            // inside one person's line ("Great, thanks. Daniel, is…") would
            // split off a piece that then takes a speaker slot of its own.
            let min_edge = MIN_SPLIT_PIECE_SECONDS;
            let mut worst_split: Option<(f32, f32)> = None;
            for &frac in fractions {
                let target = start + dur * frac;
                let split_t =
                    quietest_split_time(samples, target, start + min_edge, end - min_edge, 0.5);
                if let (Some(left), Some(right)) = (
                    extract_embedding(extractor, samples, start, split_t),
                    extract_embedding(extractor, samples, split_t, end),
                ) {
                    let sim = cosine(&left, &right);
                    if worst_split.is_none_or(|(_, best_sim)| sim < best_sim) {
                        worst_split = Some((split_t, sim));
                    }
                }
            }
            if let Some((split_t, sim)) = worst_split {
                if sim < VOICE_CHANGE_SPLIT_SIMILARITY {
                    queue.push_front((split_t, end, depth + 1));
                    queue.push_front((start, split_t, depth + 1));
                    continue;
                }
            }
        }
        out.push((start, end));
    }
    out
}

/// Whether a single-cluster diarization result has tight enough voice
/// embeddings to be confident a single person spoke (e.g. a 1:1 call), as
/// opposed to short/noisy or spread-out clips that should fall back to `"Group"`.
fn single_voice_consistent_from_embeddings(
    voices: &[Voice],
    single_half_similarity: Option<f32>,
) -> bool {
    let total_speech: f32 = voices.iter().map(|v| v.seconds.max(0.0)).sum();
    if total_speech <= 0.0 {
        return false;
    }
    let mut anchors: Vec<usize> = (0..voices.len())
        .filter(|&i| voices[i].embedding.is_some() && voices[i].seconds >= ANCHOR_SECONDS)
        .collect();
    let anchor_speech: f32 = anchors.iter().map(|&i| voices[i].seconds.max(0.0)).sum();
    if anchor_speech < 0.5 * total_speech {
        return false;
    }
    if anchors.len() > MAX_ANCHORS {
        anchors.sort_by(|&a, &b| voices[b].seconds.total_cmp(&voices[a].seconds));
        anchors.truncate(MAX_ANCHORS);
        anchors.sort_unstable();
    }
    if anchors.len() >= 2 {
        let units: Vec<Vec<f32>> = anchors
            .iter()
            .map(|&i| normalized(voices[i].embedding.as_ref().unwrap()))
            .collect();
        let mut min_cos = f32::INFINITY;
        let mut weighted_cos_sum = 0.0f32;
        let mut weight_sum = 0.0f32;
        for a in 0..units.len() {
            let wa = voices[anchors[a]].seconds.max(0.1);
            for b in a + 1..units.len() {
                let wb = voices[anchors[b]].seconds.max(0.1);
                let c: f32 = units[a].iter().zip(&units[b]).map(|(x, y)| x * y).sum();
                min_cos = min_cos.min(c);
                let w = wa * wb;
                weighted_cos_sum += w * c;
                weight_sum += w;
            }
        }
        let mean_cos = if weight_sum > 0.0 {
            weighted_cos_sum / weight_sum
        } else {
            min_cos
        };
        let min_pair_floor = SAME_SPEAKER_SIMILARITY - MAX_PAIR_SPREAD;
        return mean_cos >= SINGLE_VOICE_MIN_COSINE && min_cos >= min_pair_floor;
    }
    if anchors.len() == 1 && anchor_speech >= 0.6 * total_speech {
        return single_half_similarity.is_some_and(|sim| sim >= SINGLE_VOICE_MIN_COSINE);
    }
    false
}

/// sherpa-onnx's grouping threshold, set low so that it splits rather than
/// merges; `group_voices` does the real grouping.
const OVERSPLIT_THRESHOLD: f32 = 0.15;
const MIN_SEGMENT_SECONDS: f32 = 0.3;
/// Segments at least this long set the speakers; shorter ones (noisier
/// embeddings) then join whichever speaker they sound most like.
const ANCHOR_SECONDS: f32 = 1.2;
const FALLBACK_ANCHOR_SECONDS: f32 = 0.8;
/// Duration-weighted average pairwise similarity above which two groups are
/// taken to be the same person when the speaker count isn't given.
/// One person across >=1.2s turns scores ~0.81-0.95 with WeSpeaker ResNet34;
/// distinct voices on the same compressed call codec score up to ~0.75-0.79.
const SAME_SPEAKER_SIMILARITY: f32 = 0.792;
/// Complete-linkage guard offset: when `speakers == 0`, two groups cannot
/// merge if their worst anchor pair scores below `same_speaker - MAX_PAIR_SPREAD`,
/// preventing bridge segments from chaining distinct stand-up voices together.
const MAX_PAIR_SPREAD: f32 = 0.14;
/// Minimum pairwise cosine required to confirm a single-cluster remote track
/// belongs to one consistent speaker rather than an unseparated `"Group"`.
const SINGLE_VOICE_MIN_COSINE: f32 = 0.78;
/// Long segments are checked for missed mid-segment speaker handoffs and split
/// at a local pause valley when the two halves score below this similarity.
/// Twice the shortest piece, so both halves can reach it; any shorter and
/// `quietest_split_time`'s clamp would hand back a sub-2 s piece.
const MIN_SPLIT_SEGMENT_SECONDS: f32 = 2.0 * MIN_SPLIT_PIECE_SECONDS;
const VOICE_CHANGE_SPLIT_SIMILARITY: f32 = 0.72;
/// Shortest piece a voice-change split may leave on either side.
const MIN_SPLIT_PIECE_SECONDS: f32 = 2.0;
/// At most this many segments set the speakers (see `group_voices`).
const MAX_ANCHORS: usize = 400;

pub struct Voice {
    pub embedding: Option<Vec<f32>>,
    pub seconds: f32,
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter().map(|x| x / norm).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norms =
        a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norms == 0.0 {
        0.0
    } else {
        dot / norms
    }
}

/// Groups segments by voice, returning a group number per segment.
///
/// Agglomerative using exact duration-weighted average pairwise cosine (UPGMA)
/// plus a complete-linkage worst-pair guard: every long segment starts as its
/// own group, and the two most alike groups merge until there are `speakers`
/// groups, or, with `speakers` 0, until no eligible pair is at least
/// `same_speaker` alike. Dividing by scalar weight (`W_A * W_B`) rather than
/// re-normalizing summed centroids avoids high-dimensional centroid drift that
/// would otherwise snowball distinct call speakers into a single cluster.
/// Short segments then join the group whose mean voice direction they're
/// closest to; a segment with no embedding takes its neighbour's.
#[cfg(test)]
pub fn group_voices(voices: &[Voice], speakers: u32, same_speaker: f32) -> Vec<usize> {
    group_voices_with_hints(voices, None, speakers, same_speaker)
}

pub fn group_voices_with_hints(
    voices: &[Voice],
    ui_hints: Option<&[Option<String>]>,
    speakers: u32,
    same_speaker: f32,
) -> Vec<usize> {
    let mut anchors: Vec<usize> = (0..voices.len())
        .filter(|&i| voices[i].embedding.is_some() && voices[i].seconds >= ANCHOR_SECONDS)
        .collect();
    if anchors.is_empty() {
        anchors = (0..voices.len())
            .filter(|&i| {
                voices[i].embedding.is_some() && voices[i].seconds >= FALLBACK_ANCHOR_SECONDS
            })
            .collect();
    }
    if anchors.is_empty() {
        anchors = (0..voices.len())
            .filter(|&i| voices[i].embedding.is_some())
            .collect();
    }
    if anchors.is_empty() {
        return vec![0; voices.len()];
    }
    // Merging is quadratic in the anchor count per step; the longest
    // segments carry the clearest voices, and the rest join afterwards.
    if anchors.len() > MAX_ANCHORS {
        anchors.sort_by(|&a, &b| voices[b].seconds.total_cmp(&voices[a].seconds));
        anchors.truncate(MAX_ANCHORS);
        anchors.sort_unstable();
    }

    struct AnchorGroup {
        weighted_sum: Vec<f32>,
        total_weight: f32,
        members: Vec<usize>,
        ui_speaker: Option<String>,
    }

    let mut groups: Vec<Option<AnchorGroup>> = anchors
        .iter()
        .map(|&i| {
            let v = &voices[i];
            let weight = v.seconds.max(1e-3);
            let unit = normalized(v.embedding.as_ref().unwrap());
            let ui_speaker = ui_hints.and_then(|h| h.get(i)).and_then(|opt| opt.clone());
            Some(AnchorGroup {
                weighted_sum: unit.iter().map(|x| x * weight).collect(),
                total_weight: weight,
                members: vec![i],
                ui_speaker,
            })
        })
        .collect();

    let avg_pairwise_cosine = |g1: &AnchorGroup, g2: &AnchorGroup| -> f32 {
        let dot: f32 = g1
            .weighted_sum
            .iter()
            .zip(&g2.weighted_sum)
            .map(|(x, y)| x * y)
            .sum();
        let denom = g1.total_weight * g2.total_weight;
        if denom <= 0.0 {
            0.0
        } else {
            dot / denom
        }
    };

    // Pairwise UPGMA similarity and complete-linkage minimum similarity,
    // both updated in O(1)/O(D) per live neighbour after each merge.
    let n = groups.len();
    let mut similarity = vec![vec![f32::NEG_INFINITY; n]; n];
    let mut min_pair_sim = vec![vec![f32::INFINITY; n]; n];
    for a in 0..n {
        for b in a + 1..n {
            let s = avg_pairwise_cosine(groups[a].as_ref().unwrap(), groups[b].as_ref().unwrap());
            similarity[a][b] = s;
            similarity[b][a] = s;
            min_pair_sim[a][b] = s;
            min_pair_sim[b][a] = s;
        }
    }

    let min_allowed_pair = same_speaker - MAX_PAIR_SPREAD;
    let mut live = n;
    let target = speakers.max(1) as usize;
    while live > 1 && !(speakers > 0 && live <= target) {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..n {
            let Some(ga) = &groups[a] else {
                continue;
            };
            for b in a + 1..n {
                let Some(gb) = &groups[b] else {
                    continue;
                };
                if speakers == 0 {
                    if min_pair_sim[a][b] < min_allowed_pair {
                        continue;
                    }
                    if let (Some(ua), Some(ub)) = (&ga.ui_speaker, &gb.ui_speaker) {
                        if ua != ub {
                            continue;
                        }
                    }
                }
                if best.is_none_or(|(_, _, s)| similarity[a][b] > s) {
                    best = Some((a, b, similarity[a][b]));
                }
            }
        }
        let Some((a, b, best_similarity)) = best else {
            break;
        };
        if speakers == 0 && best_similarity < same_speaker {
            break;
        }
        let taken = groups[b].take().unwrap();
        let merged = groups[a].as_mut().unwrap();
        for (x, y) in merged.weighted_sum.iter_mut().zip(taken.weighted_sum) {
            *x += y;
        }
        merged.total_weight += taken.total_weight;
        merged.members.extend(taken.members);
        if merged.ui_speaker.is_none() {
            merged.ui_speaker = taken.ui_speaker;
        } else if taken.ui_speaker.is_some() && merged.ui_speaker != taken.ui_speaker {
            merged.ui_speaker = None;
        }
        live -= 1;
        for other in 0..n {
            if other != a {
                if let Some(go) = &groups[other] {
                    let s = avg_pairwise_cosine(groups[a].as_ref().unwrap(), go);
                    similarity[a][other] = s;
                    similarity[other][a] = s;
                    let worst = min_pair_sim[a][other].min(min_pair_sim[b][other]);
                    min_pair_sim[a][other] = worst;
                    min_pair_sim[other][a] = worst;
                }
            }
        }
    }
    let mut groups: Vec<AnchorGroup> = groups.into_iter().flatten().collect();

    // Number groups by first appearance.
    groups.sort_by_key(|g| g.members.iter().copied().min());
    let mut labels: Vec<Option<usize>> = vec![None; voices.len()];
    for (label, g) in groups.iter().enumerate() {
        for &i in &g.members {
            labels[i] = Some(label);
        }
    }
    for (i, voice) in voices.iter().enumerate() {
        if labels[i].is_none() {
            if let Some(embedding) = &voice.embedding {
                labels[i] = groups
                    .iter()
                    .enumerate()
                    .max_by(|x, y| {
                        cosine(&x.1.weighted_sum, embedding)
                            .total_cmp(&cosine(&y.1.weighted_sum, embedding))
                    })
                    .map(|(label, _)| label);
            }
        }
    }
    let mut previous = labels.iter().flatten().next().copied().unwrap_or(0);
    labels
        .into_iter()
        .map(|label| {
            let label = label.unwrap_or(previous);
            previous = label;
            label
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    pub start: f32,
    pub end: f32,
    pub speaker: i32,
}

/// Joins one speaker's back-to-back segments into turns and drops
/// fragments too short to hold words.
pub fn merge_turns(segments: &[(f32, f32, i32)]) -> Vec<Turn> {
    let mut sorted = segments.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut turns: Vec<Turn> = Vec::new();
    for (start, end, speaker) in sorted {
        if let Some(last) = turns.last_mut() {
            if last.speaker == speaker && start - last.end <= MERGE_GAP_SECONDS {
                last.end = last.end.max(end);
                continue;
            }
        }
        turns.push(Turn {
            start,
            end,
            speaker,
        });
    }
    turns.retain(|t| t.end - t.start >= MIN_TURN_SECONDS);
    turns
}

/// Merges the two channels of a stereo meeting into turns.
///
/// The channels are recorded independently, so "You" and remote speech
/// overlap all the time (backchannels, talking over each other). Merging
/// everything as one time-sorted list would split a remote turn at every
/// overlapping "yeah" from the mic, fragmenting it into short clips Whisper
/// transcribes worse, and dropping fragments under `MIN_TURN_SECONDS`.
///
/// Instead each channel merges on its own, and a turn only breaks where the
/// other channel has a real interjection *in the pause* between two of its
/// segments (one that overlaps neither), so the transcript still reads in
/// order. Turns are then interleaved by start time.
pub fn merge_dual_channel_turns(you: &[(f32, f32, i32)], remote: &[(f32, f32, i32)]) -> Vec<Turn> {
    let mut turns = merge_turns_split_by(you, remote);
    turns.extend(merge_turns_split_by(remote, you));
    turns.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.speaker.cmp(&b.speaker)));
    turns
}

/// `merge_turns` for one channel, except two segments don't merge when an
/// `other`-channel segment of at least `MIN_TURN_SECONDS` sits entirely in
/// the gap between them.
fn merge_turns_split_by(segments: &[(f32, f32, i32)], other: &[(f32, f32, i32)]) -> Vec<Turn> {
    let mut sorted = segments.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let interjects = |gap_start: f32, gap_end: f32| {
        other
            .iter()
            .any(|&(s, e, _)| s >= gap_start && e <= gap_end && e - s >= MIN_TURN_SECONDS)
    };
    let mut turns: Vec<Turn> = Vec::new();
    for (start, end, speaker) in sorted {
        if let Some(last) = turns.last_mut() {
            if last.speaker == speaker
                && start - last.end <= MERGE_GAP_SECONDS
                && !interjects(last.end, start)
            {
                last.end = last.end.max(end);
                continue;
            }
        }
        turns.push(Turn {
            start,
            end,
            speaker,
        });
    }
    turns.retain(|t| t.end - t.start >= MIN_TURN_SECONDS);
    turns
}

/// One turn's audio, padded with silence to a length Whisper accepts.
fn clip(samples: &[f32], turn: &Turn) -> Vec<f32> {
    let rate = WHISPER_SAMPLE_RATE as f32;
    let start = ((turn.start * rate) as usize).min(samples.len());
    let end = ((turn.end * rate) as usize).clamp(start, samples.len());
    let mut clip = samples[start..end].to_vec();
    let min = (MIN_CLIP_SECONDS * rate) as usize;
    if clip.len() < min {
        clip.resize(min, 0.0);
    }
    clip
}

/// The speaker whose voice matches the enrolled one, if a voice is
/// enrolled and someone matches it.
fn find_you(app: &AppHandle, samples: &[f32], turns: &[Turn]) -> Option<i32> {
    let mut best: Option<(i32, f32)> = None;
    for speaker in first_appearance_order(turns) {
        let mut audio = Vec::new();
        let limit = (YOU_SAMPLE_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
        for turn in turns.iter().filter(|t| t.speaker == speaker) {
            audio.extend(clip(samples, turn));
            if audio.len() >= limit {
                break;
            }
        }
        match crate::isolate::enrolled_speaker_similarity(app, &audio)? {
            Ok(similarity) if best.is_none_or(|(_, b)| similarity > b) => {
                best = Some((speaker, similarity))
            }
            Ok(_) => {}
            Err(err) => crate::applog!("meeting: couldn't compare voices: {err}"),
        }
    }
    best.filter(|&(_, similarity)| similarity >= crate::isolate::SIMILARITY_THRESHOLD)
        .map(|(speaker, _)| speaker)
}

fn first_appearance_order(turns: &[Turn]) -> Vec<i32> {
    let mut order = Vec::new();
    for turn in turns {
        if !order.contains(&turn.speaker) {
            order.push(turn.speaker);
        }
    }
    order
}

/// Minimum corroborated turns required to promote a UI speaker name onto a
/// diarized voice cluster (matching Wispr Flow's `promoteThreshold = 3`).
const SPEAKER_PROMOTE_TURNS: usize = 3;
/// Minimum unambiguous single-speaker overlap (seconds) within a single turn
/// for that turn to count as corroborated by the UI active-speaker border.
const MIN_TURN_OVERLAP_SECONDS: f32 = 0.30;
/// Alternative promotion threshold for a single long monologue turn where a
/// participant speaks unambiguously for at least 4 seconds.
const MIN_MONOLOGUE_OVERLAP_SECONDS: f32 = 4.0;
/// Minimum share of a cluster's total UI-observed active time that must
/// belong to the winning participant name so a multi-speaker clumped cluster
/// isn't misattributed to one participant.
const MIN_CLUSTER_OVERLAP_RATIO: f32 = 0.55;

/// Precomputes `[start_sec, end_sec]` intervals for state-change observations.
/// Each observation holds until the next one (capped at 15s, or 1.5s for the
/// final observation in the log).
fn observation_spans(
    observations: &[crate::speaker_poller::SpeakerObservation],
) -> Vec<(f32, f32, &crate::speaker_poller::SpeakerObservation)> {
    observations
        .iter()
        .enumerate()
        .map(|(i, obs)| {
            let start = obs.offset_ms as f32 / 1000.0;
            let end = observations
                .get(i + 1)
                .map(|next| (next.offset_ms as f32 / 1000.0).min(start + 15.0))
                .unwrap_or(start + 1.5)
                .max(start);
            (start, end, obs)
        })
        .collect()
}

/// Returns the participant name when Accessibility unambiguously observed one
/// non-muted active speaker dominating `[start, end]`.
fn dominant_ui_speaker<'a>(
    start: f32,
    end: f32,
    spans: &[(f32, f32, &'a crate::speaker_poller::SpeakerObservation)],
) -> Option<&'a str> {
    let dur = end - start;
    if dur < 0.7 {
        return None;
    }
    let mut active_by_name: std::collections::BTreeMap<&'a str, f32> =
        std::collections::BTreeMap::new();
    let mut muted_by_name: std::collections::BTreeMap<&'a str, f32> =
        std::collections::BTreeMap::new();
    for &(obs_start, obs_end, obs) in spans {
        let ov_start = start.max(obs_start);
        let ov_end = end.min(obs_end);
        if ov_end <= ov_start {
            continue;
        }
        let dt = ov_end - ov_start;
        for muted in &obs.muted_names {
            *muted_by_name.entry(muted.as_str()).or_insert(0.0) += dt;
        }
        if obs.speaking_names.len() == 1 {
            let active = obs.speaking_names[0].as_str();
            if !obs.muted_names.iter().any(|m| m == active) {
                *active_by_name.entry(active).or_insert(0.0) += dt;
            }
        }
    }
    let total_active: f32 = active_by_name.values().sum();
    if total_active < 0.7 {
        return None;
    }
    let (best_name, &best_dt) = active_by_name.iter().max_by(|a, b| a.1.total_cmp(b.1))?;
    let muted_dt = muted_by_name.get(best_name).copied().unwrap_or(0.0);
    if best_dt >= 0.7
        && best_dt >= 0.65 * total_active
        && best_dt >= 0.35 * dur
        && muted_dt <= 0.25 * dur
    {
        Some(*best_name)
    } else {
        None
    }
}

/// "You" for the enrolled voice, the rest "Speaker 1", "Speaker 2", ... in
/// the order they first speak.
pub fn speaker_names(turns: &[Turn], you: Option<i32>) -> std::collections::HashMap<i32, String> {
    let mut names = std::collections::HashMap::new();
    let mut next = 1;
    for speaker in first_appearance_order(turns) {
        if Some(speaker) == you {
            names.insert(speaker, "You".to_string());
        } else {
            names.insert(speaker, format!("Speaker {next}"));
            next += 1;
        }
    }
    names
}

/// Fuses timestamped UI active-speaker observations (`AXUIElement` tile
/// polling) with diarized voice clusters (`turns`) using temporal overlap
/// and muted-tile negative evidence.
///
/// Any cluster that does not meet the promotion threshold (`>= 3`
/// corroborated turns or `>= 4.0s` unambiguous overlap, and `>= 55%` share
/// of observed speaking time for that cluster) retains its default
/// `"Speaker N"` label from [`speaker_names`] — unless UI observations show
/// multiple distinct participants each speaking significantly inside that
/// cluster without a majority winner, in which case it is labeled `"Group"`.
/// The `you` cluster always remains `"You"`.
pub fn resolve_cluster_names(
    turns: &[Turn],
    observations: &[crate::speaker_poller::SpeakerObservation],
    you: Option<i32>,
) -> std::collections::HashMap<i32, String> {
    let mut names = speaker_names(turns, you);
    if observations.is_empty() || turns.is_empty() {
        return names;
    }

    let spans = observation_spans(observations);

    // BTreeMaps throughout: deterministic iteration (and float sums), so
    // re-transcribing the same meeting always picks the same names.
    // Per (speaker_id, candidate_name): (weighted_overlap_seconds, corroborated_turns)
    let mut cluster_scores: std::collections::BTreeMap<
        i32,
        std::collections::BTreeMap<String, (f32, usize)>,
    > = std::collections::BTreeMap::new();

    for speaker in first_appearance_order(turns) {
        if Some(speaker) == you {
            continue;
        }
        let mut by_name: std::collections::BTreeMap<String, (f32, usize)> =
            std::collections::BTreeMap::new();

        for turn in turns.iter().filter(|t| t.speaker == speaker) {
            let turn_dur = (turn.end - turn.start).max(0.0);
            if turn_dur <= 0.0 {
                continue;
            }
            let mut turn_active: std::collections::BTreeMap<&str, f32> =
                std::collections::BTreeMap::new();
            let mut turn_muted: std::collections::BTreeMap<&str, f32> =
                std::collections::BTreeMap::new();

            for &(obs_start, obs_end, obs) in &spans {
                let overlap_start = turn.start.max(obs_start);
                let overlap_end = turn.end.min(obs_end);
                if overlap_end <= overlap_start {
                    continue;
                }
                let dt = overlap_end - overlap_start;

                for muted in &obs.muted_names {
                    *turn_muted.entry(muted.as_str()).or_insert(0.0) += dt;
                }

                // Only unambiguous single-speaker intervals contribute positive evidence.
                if obs.speaking_names.len() == 1 {
                    let active = obs.speaking_names[0].as_str();
                    if !obs.muted_names.iter().any(|m| m == active) {
                        *turn_active.entry(active).or_insert(0.0) += dt;
                    }
                }
            }

            for (active_name, active_dt) in turn_active {
                let muted_dt = turn_muted.get(active_name).copied().unwrap_or(0.0);
                if muted_dt > 0.5 * turn_dur {
                    // Muted for the majority of the turn: penalize by 0.5x and do
                    // not count as a corroborated turn.
                    let entry = by_name.entry(active_name.to_string()).or_insert((0.0, 0));
                    entry.0 += active_dt * 0.5;
                } else {
                    let entry = by_name.entry(active_name.to_string()).or_insert((0.0, 0));
                    entry.0 += active_dt;
                    if active_dt >= MIN_TURN_OVERLAP_SECONDS {
                        entry.1 += 1;
                    }
                }
            }
        }

        if !by_name.is_empty() {
            cluster_scores.insert(speaker, by_name);
        }
    }

    // Collect eligible (speaker, name, score_seconds, turns) candidates and assign
    // greedily in descending order of score so two voice clusters never collide on
    // the same participant name.
    let mut candidates: Vec<(i32, String, f32, usize)> = Vec::new();
    let mut mixed_group_speakers = std::collections::HashSet::new();
    for (speaker, by_name) in &cluster_scores {
        let total_cluster_active: f32 = by_name.values().map(|(s, _)| *s).sum();
        if total_cluster_active <= 0.0 {
            continue;
        }
        let significant_participants = by_name
            .values()
            .filter(|&&(score_sec, corr_turns)| {
                (score_sec >= 1.2 || corr_turns >= 2) && score_sec / total_cluster_active >= 0.20
            })
            .count();
        let mut any_eligible = false;
        for (name, &(score_sec, corr_turns)) in by_name {
            let ratio = score_sec / total_cluster_active;
            let promoted =
                corr_turns >= SPEAKER_PROMOTE_TURNS || score_sec >= MIN_MONOLOGUE_OVERLAP_SECONDS;
            if promoted && ratio >= MIN_CLUSTER_OVERLAP_RATIO {
                candidates.push((*speaker, name.clone(), score_sec, corr_turns));
                any_eligible = true;
            }
        }
        if !any_eligible && significant_participants >= 2 {
            mixed_group_speakers.insert(*speaker);
        }
    }

    candidates.sort_by(|a, b| {
        b.2.total_cmp(&a.2)
            .then_with(|| b.3.cmp(&a.3))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });

    let mut assigned_speakers = std::collections::HashSet::new();
    let mut assigned_names = std::collections::HashSet::new();
    for (speaker, name, _, _) in candidates {
        if assigned_speakers.contains(&speaker) || assigned_names.contains(&name) {
            continue;
        }
        names.insert(speaker, name.clone());
        assigned_speakers.insert(speaker);
        assigned_names.insert(name);
    }

    for &speaker in &mixed_group_speakers {
        if !assigned_speakers.contains(&speaker) {
            names.insert(speaker, "Group".to_string());
            assigned_speakers.insert(speaker);
        }
    }

    // Re-number remaining unpromoted non-`you` clusters sequentially ("Speaker 1",
    // "Speaker 2", ...) in first-appearance order so promoting earlier clusters
    // never leaves a gap like "You", "Alice", "Speaker 2".
    if !assigned_speakers.is_empty() {
        let mut next_speaker_num = 1usize;
        for speaker in first_appearance_order(turns) {
            if Some(speaker) == you || assigned_speakers.contains(&speaker) {
                continue;
            }
            names.insert(speaker, format!("Speaker {next_speaker_num}"));
            next_speaker_num += 1;
        }
    }

    names
}

/// Relabels a single unpromoted remote voice bucket as `"Group"` instead of
/// `"Speaker 1"` when automatic speaker counting (`speakers == 0`) could not
/// confirm a single individual voice (or when UI tile observations saw
/// multiple active remote speakers).
///
/// Prevents a group stand-up or call whose voices couldn't be separated from
/// misleadingly reading as one person (`"Speaker 1"`).
pub fn apply_remote_group_fallback(
    names: &mut std::collections::HashMap<i32, String>,
    turns: &[Turn],
    observations: &[crate::speaker_poller::SpeakerObservation],
    configured_speakers: u32,
    confirmed_single_voice: bool,
) {
    if configured_speakers > 0 {
        return;
    }
    let remote_ids: Vec<i32> = first_appearance_order(turns)
        .into_iter()
        .filter(|&id| id != YOU_SPEAKER_ID)
        .collect();
    let [only_remote] = remote_ids.as_slice() else {
        return;
    };
    let is_generic_label = names
        .get(only_remote)
        .is_some_and(|label| label.starts_with("Speaker "));
    if !is_generic_label {
        return;
    }
    let spans = observation_spans(observations);
    let mut ui_overlap_by_speaker: std::collections::HashMap<&str, f32> =
        std::collections::HashMap::new();
    for turn in turns.iter().filter(|t| t.speaker == *only_remote) {
        for &(obs_start, obs_end, obs) in &spans {
            let dt = (turn.end.min(obs_end) - turn.start.max(obs_start)).max(0.0);
            if dt <= 0.0 {
                continue;
            }
            for speaking in &obs.speaking_names {
                if !obs.muted_names.iter().any(|m| m == speaking) {
                    *ui_overlap_by_speaker
                        .entry(speaking.as_str())
                        .or_insert(0.0) += dt;
                }
            }
        }
    }
    let active_ui_speakers = ui_overlap_by_speaker
        .values()
        .filter(|&&secs| secs >= MIN_TURN_OVERLAP_SECONDS)
        .count();
    if active_ui_speakers >= 2 || !confirmed_single_voice {
        names.insert(*only_remote, "Group".to_string());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub start: f32,
    pub speaker: String,
    pub text: String,
}

/// The end of what's been said so far, for Whisper's prompt.
fn recent_context(lines: &[Line]) -> String {
    let joined = lines
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let skip = joined.chars().count().saturating_sub(200);
    joined.chars().skip(skip).collect()
}

fn meeting_title(started: chrono::DateTime<chrono::Local>) -> String {
    format!("Meeting, {}", started.format("%a %-d %b %Y, %H:%M"))
}

fn timestamp(seconds: f32) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn meeting_detail(duration_seconds: f32, lines: &[Line]) -> String {
    let minutes = (duration_seconds / 60.0).round().max(1.0) as u64;
    let speakers: Vec<&str> = lines.iter().map(|l| l.speaker.as_str()).collect();
    let people = people_label_from_speakers(&speakers);
    format!("{minutes} min · {people}")
}

fn people_label_from_speakers(raw_speakers: &[&str]) -> String {
    let mut speakers = raw_speakers.to_vec();
    speakers.sort_unstable();
    speakers.dedup();
    let has_group = speakers.contains(&"Group");
    if has_group {
        let individuals = speakers.iter().filter(|&&s| s != "Group").count();
        match individuals {
            0 => "group".to_string(),
            1 => {
                let solo = speakers
                    .iter()
                    .copied()
                    .find(|&s| s != "Group")
                    .unwrap_or("You");
                format!("{solo} + group")
            }
            n => format!("{}+ speakers", n + 1),
        }
    } else {
        match speakers.len() {
            0 | 1 => "1 speaker".to_string(),
            n => format!("{n} speakers"),
        }
    }
}

/// The transcript: one paragraph per turn, with consecutive turns from the
/// same speaker joined.
pub fn render_markdown(title: &str, detail: &str, lines: &[Line]) -> String {
    let mut out = format!("# {title}\n\n{detail}");
    let mut previous: Option<String> = None;
    for line in lines {
        // Names can come from the call app's window; a `*` or line break
        // in one would break the `**Name** · 0:00` turn header.
        let mut speaker = clean_speaker_name(&line.speaker);
        if speaker.is_empty() {
            speaker = "Speaker".to_string();
        }
        if previous.as_deref() == Some(speaker.as_str()) {
            out.push(' ');
            out.push_str(&line.text);
        } else {
            out.push_str(&format!(
                "\n\n**{}** · {}\n{}",
                speaker,
                timestamp(line.start),
                line.text
            ));
            previous = Some(speaker);
        }
    }
    out.push('\n');
    out
}

/// A speaker name safe to put in a `**Name** · 0:00` turn header: no `*`,
/// line breaks or other control characters.
fn clean_speaker_name(name: &str) -> String {
    name.chars()
        .filter(|&c| c != '*' && !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedTurn {
    pub speaker: String,
    pub timestamp: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedMeetingDoc {
    pub title: String,
    pub detail: String,
    pub notes: Option<String>,
    pub turns: Vec<ParsedTurn>,
}

fn parse_turn_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("**")?;
    let sep = rest.rfind("** · ")?;
    let speaker = rest[..sep].trim();
    let ts = rest[sep + "** · ".len()..].trim();
    if speaker.is_empty() || ts.is_empty() {
        return None;
    }
    let parts: Vec<&str> = ts.split(':').collect();
    if !(2..=3).contains(&parts.len())
        || parts[0].is_empty()
        || !parts[0].chars().all(|c| c.is_ascii_digit())
        || !parts[1..]
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some((speaker.to_string(), ts.to_string()))
}

pub fn parse_meeting_markdown(markdown: &str) -> ParsedMeetingDoc {
    let mut title = String::from("Meeting");
    let mut detail = String::new();
    let mut notes_lines: Vec<String> = Vec::new();
    let mut turns: Vec<ParsedTurn> = Vec::new();
    let mut current_turn: Option<ParsedTurn> = None;
    let mut saw_title = false;
    let mut saw_detail = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if !saw_title {
            if trimmed.is_empty() {
                continue;
            }
            if let Some(t) = trimmed.strip_prefix("# ") {
                title = t.trim().to_string();
                saw_title = true;
                continue;
            }
            saw_title = true;
        }
        if !saw_detail && current_turn.is_none() && notes_lines.is_empty() {
            if trimmed.is_empty() {
                continue;
            }
            if !trimmed.starts_with("## ") && parse_turn_header(trimmed).is_none() {
                detail = trimmed.to_string();
                saw_detail = true;
                continue;
            }
            saw_detail = true;
        }

        if let Some((speaker, timestamp)) = parse_turn_header(trimmed) {
            if let Some(mut finished) = current_turn.take() {
                finished.text = finished.text.trim().to_string();
                if !finished.text.is_empty() {
                    turns.push(finished);
                }
            }
            current_turn = Some(ParsedTurn {
                speaker,
                timestamp,
                text: String::new(),
            });
            continue;
        }

        if let Some(ref mut turn) = current_turn {
            if !turn.text.is_empty() {
                turn.text.push('\n');
            }
            turn.text.push_str(line);
        } else if trimmed != "## Notes" && (!trimmed.is_empty() || !notes_lines.is_empty()) {
            notes_lines.push(line.to_string());
        }
    }

    if let Some(mut finished) = current_turn.take() {
        finished.text = finished.text.trim().to_string();
        if !finished.text.is_empty() {
            turns.push(finished);
        }
    }

    let notes_joined = notes_lines.join("\n").trim().to_string();
    let notes = (!notes_joined.is_empty()).then_some(notes_joined);

    ParsedMeetingDoc {
        title,
        detail,
        notes,
        turns,
    }
}

impl ParsedMeetingDoc {
    pub fn to_markdown(&self) -> String {
        let mut out = format!("# {}\n\n{}", self.title, self.detail);
        if let Some(notes) = self
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            out.push_str(&format!("\n\n## Notes\n{notes}"));
        }
        for turn in &self.turns {
            out.push_str(&format!(
                "\n\n**{}** · {}\n{}",
                turn.speaker,
                turn.timestamp,
                turn.text.trim()
            ));
        }
        out.push('\n');
        out
    }

    pub fn transcript_only(&self) -> String {
        self.turns
            .iter()
            .map(|t| format!("{} ({}): {}", t.speaker, t.timestamp, t.text.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn rename_speaker(&mut self, old_speaker: &str, new_speaker: &str) -> bool {
        let old_trimmed = old_speaker.trim();
        let new_trimmed = clean_speaker_name(new_speaker);
        if old_trimmed.is_empty() || new_trimmed.is_empty() || old_trimmed == new_trimmed {
            return false;
        }
        let mut changed = false;
        for turn in &mut self.turns {
            if turn.speaker == old_trimmed {
                turn.speaker = new_trimmed.clone();
                changed = true;
            }
        }
        if !changed {
            return false;
        }
        let mut merged: Vec<ParsedTurn> = Vec::with_capacity(self.turns.len());
        for turn in self.turns.drain(..) {
            if let Some(prev) = merged.last_mut().filter(|p| p.speaker == turn.speaker) {
                if !prev.text.ends_with(' ') && !turn.text.is_empty() {
                    prev.text.push(' ');
                }
                prev.text.push_str(turn.text.trim());
            } else {
                merged.push(turn);
            }
        }
        self.turns = merged;
        if let Some((duration_prefix, _)) = self.detail.split_once(" · ") {
            let speakers: Vec<&str> = self.turns.iter().map(|t| t.speaker.as_str()).collect();
            let people = people_label_from_speakers(&speakers);
            self.detail = format!("{duration_prefix} · {people}");
        }
        true
    }
}

// --------------------------------------------------------------- listing

/// Title and detail are the transcript's first lines, so only those are read.
fn header_lines(path: &Path) -> Option<Vec<String>> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    Some(
        std::io::BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .take(2)
            .collect(),
    )
}

fn summarize(dir: &Path, id: &str, status: &MeetingStatus) -> MeetingSummary {
    let header = header_lines(&dir.join(format!("{id}.md")));
    let state = if status.recording.as_ref().is_some_and(|r| r.id == id) {
        MeetingFileState::Recording
    } else if status.queued.iter().any(|q| q == id) {
        MeetingFileState::Processing
    } else if header.is_some() {
        MeetingFileState::Ready
    } else {
        MeetingFileState::NotTranscribed
    };
    let mut lines = header.unwrap_or_default().into_iter();
    let title = lines
        .next()
        .and_then(|l| l.strip_prefix("# ").map(str::to_string))
        .or_else(|| started_at(id).map(meeting_title))
        .unwrap_or_else(|| id.to_string());
    let detail = lines
        .next()
        .filter(|l| !l.starts_with("## ") && parse_turn_header(l).is_none());
    MeetingSummary {
        id: id.to_string(),
        title,
        detail,
        state,
    }
}

fn list(app: &AppHandle) -> Result<Vec<MeetingSummary>, String> {
    let dir = meetings_dir(app)?;
    let status = status(app);
    let mut ids: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let ext = path.extension()?.to_str()?;
            (ext == "md" || ext == "wav")
                .then(|| path.file_stem()?.to_str().map(str::to_string))
                .flatten()
        })
        .filter(|id| valid_id(id))
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.dedup();
    Ok(ids.iter().map(|id| summarize(&dir, id, &status)).collect())
}

// -------------------------------------------------------------- commands

#[tauri::command]
pub fn start_meeting(app: AppHandle) -> Result<(), String> {
    start(&app)
}

#[tauri::command]
pub fn stop_meeting(app: AppHandle) -> Result<(), String> {
    stop(&app)
}

#[tauri::command]
pub fn get_meeting_status(app: AppHandle) -> MeetingStatus {
    status(&app)
}

#[tauri::command]
pub async fn list_meetings(app: AppHandle) -> Result<Vec<MeetingSummary>, String> {
    crate::privacy::require_history_unlocked(&app)?;
    list(&app)
}

#[tauri::command]
pub async fn read_meeting(app: AppHandle, id: String) -> Result<String, String> {
    crate::privacy::require_history_unlocked(&app)?;
    let path = meetings_dir(&app)?.join(format!("{}.md", checked_id(&id)?));
    std::fs::read_to_string(path).map_err(|_| "That meeting has no transcript yet.".to_string())
}

#[tauri::command]
pub async fn rename_meeting_title(
    app: AppHandle,
    id: String,
    title: String,
) -> Result<MeetingSummary, String> {
    crate::privacy::require_history_unlocked(&app)?;
    let id = checked_id(&id)?;
    let clean_title: String = title
        .trim()
        .chars()
        .filter(|&c| c != '\n' && c != '\r')
        .collect::<String>()
        .trim()
        .to_string();
    if clean_title.is_empty() {
        return Err("Meeting title cannot be empty.".into());
    }
    let dir = meetings_dir(&app)?;
    let path = dir.join(format!("{id}.md"));
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| "That meeting has no transcript yet.".to_string())?;
    let mut doc = parse_meeting_markdown(&raw);
    doc.title = clean_title;
    write_private(&path, &doc.to_markdown())?;
    announce(&app);
    Ok(summarize(&dir, id, &status(&app)))
}

#[tauri::command]
pub async fn rename_meeting_speaker(
    app: AppHandle,
    id: String,
    old_speaker: String,
    new_speaker: String,
) -> Result<String, String> {
    crate::privacy::require_history_unlocked(&app)?;
    let id = checked_id(&id)?;
    let dir = meetings_dir(&app)?;
    let path = dir.join(format!("{id}.md"));
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| "That meeting has no transcript yet.".to_string())?;
    let mut doc = parse_meeting_markdown(&raw);
    if !doc.rename_speaker(&old_speaker, &new_speaker) {
        return Ok(raw);
    }
    let updated = doc.to_markdown();
    write_private(&path, &updated)?;
    announce(&app);
    Ok(updated)
}

#[tauri::command]
pub async fn summarize_meeting(app: AppHandle, id: String) -> Result<String, String> {
    crate::privacy::require_history_unlocked(&app)?;
    let id = checked_id(&id)?.to_string();
    let dir = meetings_dir(&app)?;
    let path = dir.join(format!("{id}.md"));
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| "That meeting has no transcript yet.".to_string())?;
    let doc = parse_meeting_markdown(&raw);
    if doc.turns.is_empty() {
        return Err("This meeting has no speaker turns to summarize.".into());
    }
    let cfg = crate::config::load(&app);
    let transcript_text = doc.transcript_only();
    let notes = tauri::async_runtime::spawn_blocking(move || {
        crate::llm::summarize_meeting_transcript(&transcript_text, &cfg.llm_model, &cfg.vocabulary)
    })
    .await
    .map_err(|e| format!("Summarization task failed: {e}"))??;

    // Re-read the file before writing in case the user renamed a speaker or
    // title while the LLM was generating.
    // If it's gone, the user deleted it meanwhile; writing the notes would
    // bring it back.
    let latest_raw =
        std::fs::read_to_string(&path).map_err(|_| "That meeting was deleted.".to_string())?;
    let mut latest_doc = parse_meeting_markdown(&latest_raw);
    latest_doc.notes = Some(notes);
    let updated = latest_doc.to_markdown();
    write_private(&path, &updated)?;
    announce(&app);
    Ok(updated)
}

/// Copies the transcript from here, not the webview: the widget window is
/// never focused, so the browser clipboard API refuses it.
#[tauri::command]
pub async fn copy_meeting(app: AppHandle, id: String) -> Result<(), String> {
    crate::privacy::require_history_unlocked(&app)?;
    let path = meetings_dir(&app)?.join(format!("{}.md", checked_id(&id)?));
    let text = std::fs::read_to_string(path)
        .map_err(|_| "That meeting has no transcript yet.".to_string())?;
    crate::paste::copy_text(&text)
}

/// Opens the transcript in the default text editor (TextEdit unless
/// changed); plain `open` might pick Xcode or nothing for `.md`.
#[tauri::command]
pub async fn open_meeting(app: AppHandle, id: String) -> Result<(), String> {
    crate::privacy::require_history_unlocked(&app)?;
    let path = meetings_dir(&app)?.join(format!("{}.md", checked_id(&id)?));
    if !path.exists() {
        return Err("That meeting has no transcript yet.".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&path)
        .status()
        .map_err(|e| e.to_string())
        .and_then(|s| {
            s.success()
                .then_some(())
                .ok_or_else(|| "couldn't open the transcript".to_string())
        })
}

#[tauri::command]
pub async fn delete_meeting(app: AppHandle, id: String) -> Result<(), String> {
    crate::privacy::require_history_unlocked(&app)?;
    let id = checked_id(&id)?;
    let recording = status(&app).recording.is_some_and(|r| r.id == id);
    if recording || is_queued(&app, id) {
        return Err("That meeting is still being recorded or transcribed.".into());
    }
    let dir = meetings_dir(&app)?;
    for ext in ["md", "wav", "obs.json"] {
        let path = dir.join(format!("{id}.{ext}"));
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    announce(&app);
    Ok(())
}

/// Transcribes a recording that doesn't have a transcript yet.
#[tauri::command]
pub async fn transcribe_meeting(app: AppHandle, id: String) -> Result<(), String> {
    crate::privacy::require_history_unlocked(&app)?;
    let id = checked_id(&id)?.to_string();
    if !meetings_dir(&app)?.join(format!("{id}.wav")).exists() {
        return Err("That meeting's recording is gone.".into());
    }
    if !enqueue(&app, &id) {
        return Err("That meeting is already being transcribed.".into());
    }
    announce(&app);
    std::thread::spawn(move || transcribe_saved(&app, &id));
    Ok(())
}

#[tauri::command]
pub fn get_meeting_speakers(app: AppHandle) -> u32 {
    crate::config::load(&app).meeting_speakers
}

#[tauri::command]
pub fn set_meeting_speakers(app: AppHandle, speakers: u32) -> Result<(), String> {
    let mut cfg = crate::config::load(&app);
    cfg.meeting_speakers = speakers.min(16);
    crate::config::save(&app, &cfg)
}

#[tauri::command]
pub fn get_meeting_auto_detect(app: AppHandle) -> bool {
    crate::config::load(&app).meeting_auto_detect
}

#[tauri::command]
pub fn set_meeting_auto_detect(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut cfg = crate::config::load(&app);
    cfg.meeting_auto_detect = enabled;
    crate::config::save(&app, &cfg)?;
    if !enabled {
        let state = app.state::<MeetingState>();
        lock(&state.auto_detect).dismiss_start_prompt();
        let _ = app.emit("meeting-detect-cleared", ());
    }
    Ok(())
}

#[tauri::command]
pub fn dismiss_meeting_prompt(app: AppHandle) {
    let state = app.state::<MeetingState>();
    lock(&state.auto_detect).dismiss_start_prompt();
    let _ = app.emit("meeting-detect-cleared", ());
}

#[tauri::command]
pub fn get_detected_meeting(app: AppHandle) -> Option<crate::meeting_detect::DetectedCall> {
    if !crate::config::load(&app).meeting_auto_detect || is_recording(&app) {
        return None;
    }
    let state = app.state::<MeetingState>();
    let detected = lock(&state.auto_detect).prompted_call().cloned();
    detected
}

/// Spawns the lightweight background poller (every 2s) that inspects CoreAudio
/// process holders and browser call window titles to offer non-intrusive
/// "Record <App>?" and "Call ended · Stop?" prompts in the widget.
pub fn spawn_auto_detect(app: AppHandle) {
    use crate::meeting_detect::AutoDetectAction;
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        if !crate::config::load(&app).meeting_auto_detect {
            // Forget bindings/latches, or re-enabling later could suppress
            // the prompt for a new call on the same app.
            if let Some(state) = app.try_state::<MeetingState>() {
                lock(&state.auto_detect).reset();
            }
            continue;
        }
        let meeting_recording = is_recording(&app);
        // While recording, a muted user's call app (or browser) stops holding
        // the mic but keeps playing output; requiring mic input here would
        // prompt "Call ended" in the middle of a live call.
        let detected = if meeting_recording {
            crate::meeting_detect::detect_active_call_for_poller()
        } else {
            crate::meeting_detect::detect_active_call()
        };
        let sys_silent = current_system_audio_level(&app) < 0.005;

        let action = {
            let Some(state) = app.try_state::<MeetingState>() else {
                continue;
            };
            let mut tracker = lock(&state.auto_detect);
            tracker.observe(detected, meeting_recording, sys_silent)
        };

        match action {
            AutoDetectAction::None => {}
            AutoDetectAction::PromptStart(call) => {
                crate::applog!(
                    "meeting: auto-detected active {} call ({})",
                    call.platform.label(),
                    call.bundle_id
                );
                crate::widget::show(&app);
                let _ = app.emit("meeting-detected", call);
            }
            AutoDetectAction::ClearStartPrompt => {
                let _ = app.emit("meeting-detect-cleared", ());
            }
            AutoDetectAction::PromptStop(call) => {
                crate::applog!(
                    "meeting: detected {} call ended ({}); prompting to stop recording",
                    call.platform.label(),
                    call.bundle_id
                );
                crate::widget::show(&app);
                let _ = app.emit("meeting-ended-detected", call);
            }
            AutoDetectAction::ClearStopPrompt => {
                let _ = app.emit("meeting-ended-cleared", ());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_from_one_speaker_merge_into_turns() {
        let turns = merge_turns(&[
            (0.0, 2.0, 0),
            (2.5, 4.0, 0),
            (4.2, 6.0, 1),
            (9.0, 10.0, 1),
            (10.2, 10.4, 0),
        ]);
        assert_eq!(
            turns,
            vec![
                Turn {
                    start: 0.0,
                    end: 4.0,
                    speaker: 0
                },
                Turn {
                    start: 4.2,
                    end: 6.0,
                    speaker: 1
                },
                Turn {
                    start: 9.0,
                    end: 10.0,
                    speaker: 1
                },
            ],
            "a 3s pause splits the turn; a 0.2s fragment is dropped"
        );
    }

    #[test]
    fn unsorted_segments_are_ordered_first() {
        let turns = merge_turns(&[(5.0, 6.0, 1), (0.0, 1.0, 0), (1.5, 3.0, 0)]);
        assert_eq!(
            turns[0],
            Turn {
                start: 0.0,
                end: 3.0,
                speaker: 0
            }
        );
        assert_eq!(turns[1].speaker, 1);
    }

    #[test]
    fn speakers_are_numbered_by_first_appearance_and_you_is_named() {
        let turns = merge_turns(&[(0.0, 1.0, 3), (2.0, 3.0, 0), (4.0, 5.0, 7), (6.0, 7.0, 3)]);
        let names = speaker_names(&turns, None);
        assert_eq!(names[&3], "Speaker 1");
        assert_eq!(names[&0], "Speaker 2");
        assert_eq!(names[&7], "Speaker 3");
        let names = speaker_names(&turns, Some(0));
        assert_eq!(names[&3], "Speaker 1");
        assert_eq!(names[&0], "You");
        assert_eq!(names[&7], "Speaker 2");
    }

    #[test]
    fn markdown_joins_a_speakers_consecutive_turns() {
        let line = |start, speaker: &str, text: &str| Line {
            start,
            speaker: speaker.into(),
            text: text.into(),
        };
        let lines = [
            line(0.0, "You", "Shall we start?"),
            line(3.0, "Speaker 1", "Sure."),
            line(65.0, "Speaker 1", "The release is Friday."),
            line(3725.0, "You", "Thanks, all."),
        ];
        let detail = meeting_detail(3730.0, &lines);
        assert_eq!(detail, "62 min · 2 speakers");
        assert_eq!(
            render_markdown("Meeting, Mon 26 Oct 2026, 17:40", &detail, &lines),
            "# Meeting, Mon 26 Oct 2026, 17:40\n\n62 min · 2 speakers\n\n\
             **You** · 00:00\nShall we start?\n\n\
             **Speaker 1** · 00:03\nSure. The release is Friday.\n\n\
             **You** · 1:02:05\nThanks, all.\n"
        );
    }

    fn voice(direction: &[f32], seconds: f32) -> Voice {
        Voice {
            embedding: Some(direction.to_vec()),
            seconds,
        }
    }

    #[test]
    fn voices_group_by_similarity_when_the_count_is_unknown() {
        let voices = [
            voice(&[1.0, 0.0, 0.1], 3.0),
            voice(&[0.0, 1.0, 0.0], 2.0),
            voice(&[0.95, 0.05, 0.0], 4.0),
            voice(&[0.1, 0.9, 0.1], 2.5),
            Voice {
                embedding: None,
                seconds: 0.4,
            },
            voice(&[0.9, 0.0, 0.2], 0.6),
        ];
        assert_eq!(group_voices(&voices, 0, 0.6), vec![0, 1, 0, 1, 1, 0]);
    }

    #[test]
    fn a_given_speaker_count_splits_close_voices() {
        // Two similar voices (0.8 alike) and a distinct one.
        let a = [1.0, 0.0, 0.0];
        let b = [0.8, 0.6, 0.0];
        let c = [0.0, 0.0, 1.0];
        let voices = [
            voice(&a, 2.0),
            voice(&c, 2.0),
            voice(&b, 2.0),
            voice(&a, 2.0),
            voice(&b, 2.0),
        ];
        assert_eq!(group_voices(&voices, 0, 0.6), vec![0, 1, 0, 0, 0]);
        assert_eq!(group_voices(&voices, 3, 0.6), vec![0, 1, 2, 0, 2]);
    }

    #[test]
    fn no_usable_voices_is_one_speaker() {
        let voices = [
            Voice {
                embedding: None,
                seconds: 2.0,
            },
            Voice {
                embedding: None,
                seconds: 1.0,
            },
        ];
        assert_eq!(group_voices(&voices, 0, 0.6), vec![0, 0]);
        assert!(group_voices(&[], 2, 0.6).is_empty());
    }

    #[test]
    fn short_turns_are_padded_for_whisper() {
        let samples = vec![0.5; 16_000 * 3];
        let short = clip(
            &samples,
            &Turn {
                start: 1.0,
                end: 1.6,
                speaker: 0,
            },
        );
        assert_eq!(short.len(), (MIN_CLIP_SECONDS * 16_000.0) as usize);
        assert_eq!(short[0], 0.5);
        assert_eq!(*short.last().unwrap(), 0.0);
        // A turn running past the end is cut at the end.
        let tail = clip(
            &samples,
            &Turn {
                start: 1.0,
                end: 9.0,
                speaker: 0,
            },
        );
        assert_eq!(tail.len(), 32_000);
    }

    #[test]
    fn audio_is_converted_to_16k_mono_without_losing_samples() {
        let mut to16k = To16k::default();
        let mut out = Vec::new();
        // 1 second of 48 kHz stereo in uneven chunks.
        for size in [700usize, 1300, 48_000 * 2 - 2000] {
            to16k.push(
                &AudioChunk {
                    samples: vec![0.25; size],
                    channels: 2,
                    sample_rate: 48_000,
                },
                &mut out,
            );
        }
        to16k.finish(&mut out);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn a_mic_switch_mid_meeting_keeps_both_parts() {
        let mut to16k = To16k::default();
        let mut out = Vec::new();
        let chunk = |rate: u32, frames: usize| AudioChunk {
            samples: vec![0.1; frames],
            channels: 1,
            sample_rate: rate,
        };
        to16k.push(&chunk(48_000, 24_000), &mut out);
        to16k.push(&chunk(44_100, 44_100), &mut out);
        to16k.finish(&mut out);
        assert_eq!(out.len(), 8_000 + 16_000);
    }

    #[test]
    fn ids_are_plain_file_stems() {
        let id = new_id(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 26)
                .unwrap()
                .and_hms_opt(17, 40, 5)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap(),
        );
        assert_eq!(id, "2026-10-26-174005");
        assert!(valid_id(&id));
        for bad in ["", "../x", "a.md", "2026/10", &"1".repeat(40)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn grouping_thousands_of_segments_stays_quick() {
        // Six speakers, 3,000 segments: a long meeting's worth.
        let voices: Vec<Voice> = (0..3000)
            .map(|i| {
                let mut v = vec![0.0f32; 256];
                v[i % 6] = 1.0;
                v[(i * 7) % 256] += 0.2;
                voice(&v, 1.0 + (i % 5) as f32)
            })
            .collect();
        let started = std::time::Instant::now();
        let labels = group_voices(&voices, 6, 0.6);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        for i in 0..voices.len() {
            assert_eq!(labels[i], labels[i % 6], "segment {i}");
        }
    }

    #[test]
    fn long_turns_are_cut_into_even_pieces() {
        let rate = WHISPER_SAMPLE_RATE as usize;
        let lens = |secs: usize| {
            pieces(&vec![0.0; secs * rate])
                .map(|p| p.len() / rate)
                .collect::<Vec<_>>()
        };
        assert_eq!(lens(20), vec![20]);
        assert_eq!(lens(70), vec![23, 23, 23]);
        assert_eq!(pieces(&[]).count(), 0);
    }

    #[test]
    fn a_second_meeting_in_the_same_second_gets_its_own_id() {
        let dir = std::env::temp_dir().join(format!("mp-meeting-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 26)
            .unwrap()
            .and_hms_opt(17, 40, 5)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .unwrap();
        let (first, _) = create_recording(&dir, now).unwrap();
        let (second, _) = create_recording(&dir, now).unwrap();
        // A transcript whose recording is gone still holds its id.
        std::fs::write(dir.join("2026-10-26-174005-3.md"), "# x").unwrap();
        let (third, _) = create_recording(&dir, now).unwrap();
        assert_eq!(
            [first.as_str(), second.as_str(), third.as_str()],
            [
                "2026-10-26-174005",
                "2026-10-26-174005-2",
                "2026-10-26-174005-4"
            ]
        );
        assert!(started_at(&third).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_recording_survives_a_mic_switch_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("mp-meeting-wav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.wav");
        let file = std::fs::File::create(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let writer = std::thread::spawn(move || write_meeting(file, rx));
        tx.send(AudioChunk {
            samples: vec![0.5; 96_000],
            channels: 2,
            sample_rate: 48_000,
        })
        .unwrap();
        tx.send(AudioChunk {
            samples: vec![-0.5; 44_100],
            channels: 1,
            sample_rate: 44_100,
        })
        .unwrap();
        drop(tx);
        writer.join().unwrap().unwrap();
        let samples = load_recording(&path).unwrap();
        assert_eq!(samples.len(), 32_000);
        assert!((samples[0] - 0.5).abs() < 1e-3 && (samples[31_999] + 0.5).abs() < 1e-3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(0.0), "00:00");
        assert_eq!(timestamp(65.4), "01:05");
        assert_eq!(timestamp(3725.0), "1:02:05");
    }

    #[test]
    fn resolve_cluster_names_promotes_after_three_corroborated_turns() {
        use crate::speaker_poller::SpeakerObservation;
        let turns = vec![
            Turn {
                start: 1.0,
                end: 2.5,
                speaker: 0,
            },
            Turn {
                start: 4.0,
                end: 5.5,
                speaker: 0,
            },
            Turn {
                start: 8.0,
                end: 9.5,
                speaker: 0,
            },
        ];
        let observations = vec![
            SpeakerObservation {
                offset_ms: 1100,
                speaking_names: vec!["Alice Chen".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 2400,
                speaking_names: vec![],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 4100,
                speaking_names: vec!["Alice Chen".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 5400,
                speaking_names: vec![],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 8100,
                speaking_names: vec!["Alice Chen".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 9400,
                speaking_names: vec![],
                muted_names: vec![],
            },
        ];
        let names = resolve_cluster_names(&turns, &observations, None);
        assert_eq!(names[&0], "Alice Chen");
    }

    #[test]
    fn resolve_cluster_names_rejects_single_turn_ui_flicker() {
        use crate::speaker_poller::SpeakerObservation;
        let turns = vec![
            Turn {
                start: 1.0,
                end: 2.5,
                speaker: 0,
            },
            Turn {
                start: 4.0,
                end: 5.5,
                speaker: 0,
            },
        ];
        // Only a single short highlight on turn 1 (< 3 turns and < 4.0s).
        let observations = vec![
            SpeakerObservation {
                offset_ms: 1200,
                speaking_names: vec!["Bob".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 1800,
                speaking_names: vec![],
                muted_names: vec![],
            },
        ];
        let names = resolve_cluster_names(&turns, &observations, None);
        assert_eq!(names[&0], "Speaker 1");
    }

    #[test]
    fn resolve_cluster_names_penalizes_muted_participants() {
        use crate::speaker_poller::SpeakerObservation;
        let turns = vec![
            Turn {
                start: 1.0,
                end: 2.5,
                speaker: 0,
            },
            Turn {
                start: 4.0,
                end: 5.5,
                speaker: 0,
            },
            Turn {
                start: 8.0,
                end: 9.5,
                speaker: 0,
            },
        ];
        // Charlie's tile flickers active on all 3 turns, but Charlie is muted!
        let observations = vec![
            SpeakerObservation {
                offset_ms: 1000,
                speaking_names: vec!["Charlie".into()],
                muted_names: vec!["Charlie".into()],
            },
            SpeakerObservation {
                offset_ms: 9500,
                speaking_names: vec![],
                muted_names: vec!["Charlie".into()],
            },
        ];
        let names = resolve_cluster_names(&turns, &observations, None);
        assert_eq!(names[&0], "Speaker 1");
    }

    #[test]
    fn resolve_cluster_names_preserves_you_and_falls_back_for_unmatched() {
        use crate::speaker_poller::SpeakerObservation;
        let turns = vec![
            Turn {
                start: 0.0,
                end: 2.0,
                speaker: -1,
            },
            Turn {
                start: 3.0,
                end: 8.5,
                speaker: 0,
            },
            Turn {
                start: 10.0,
                end: 12.0,
                speaker: 1,
            },
        ];
        // Cluster 0 has a 5.0s unambiguous monologue as "Dana" (>= 4.0s monologue promotion),
        // while Cluster 1 has no observations and Cluster -1 is `you`.
        let observations = vec![
            SpeakerObservation {
                offset_ms: 3200,
                speaking_names: vec!["Dana".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 8200,
                speaking_names: vec![],
                muted_names: vec![],
            },
        ];
        let names = resolve_cluster_names(&turns, &observations, Some(-1));
        assert_eq!(names[&-1], "You");
        assert_eq!(names[&0], "Dana");
        assert_eq!(names[&1], "Speaker 1");
    }

    #[test]
    fn stereo_recording_writes_and_deinterleaves_mic_and_system_channels() {
        let dir =
            std::env::temp_dir().join(format!("mp-meeting-stereo-wav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stereo.wav");
        let file = std::fs::File::create(&path).unwrap();
        let (mic_tx, mic_rx) = mpsc::channel();
        let (sys_tx, sys_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || write_meeting_stereo(file, mic_rx, sys_rx));

        // Send 1 second of 16 kHz audio on both channels: mic = +0.4, system = -0.6
        sys_tx
            .send(AudioChunk {
                samples: vec![-0.6; 16_000],
                channels: 1,
                sample_rate: 16_000,
            })
            .unwrap();
        mic_tx
            .send(AudioChunk {
                samples: vec![0.4; 16_000],
                channels: 1,
                sample_rate: 16_000,
            })
            .unwrap();
        drop(sys_tx);
        drop(mic_tx);
        writer.join().unwrap().unwrap();

        let loaded = load_recording_channels(&path).unwrap();
        let sys = loaded.system.expect("expected stereo system channel");
        assert_eq!(loaded.mic.len(), 16_000);
        assert_eq!(sys.len(), 16_000);
        assert!((loaded.mic[0] - 0.4).abs() < 1e-3);
        assert!((sys[0] + 0.6).abs() < 1e-3);

        // Also verify that staggered sub-100ms chunk arrivals (where `mic_to16k`
        // crosses 100ms block boundaries slightly before `sys_to16k`) do not
        // splice zero-gaps into the system channel or shift stereo phase.
        let staggered_path = dir.join("staggered.wav");
        let staggered_file = std::fs::File::create(&staggered_path).unwrap();
        let (m_tx, m_rx) = mpsc::channel();
        let (s_tx, s_rx) = mpsc::channel();
        let staggered_writer =
            std::thread::spawn(move || write_meeting_stereo(staggered_file, m_rx, s_rx));
        for step in 0..20 {
            // Mic sends 800 samples (50ms) first; system sends 800 samples on
            // odd steps (1,600 samples every 2 steps) so `mic_to16k` crosses
            // each 1,600-sample boundary before `sys_to16k`.
            m_tx.send(AudioChunk {
                samples: vec![0.35; 800],
                channels: 1,
                sample_rate: 16_000,
            })
            .unwrap();
            if step % 2 == 1 {
                s_tx.send(AudioChunk {
                    samples: vec![-0.45; 1_600],
                    channels: 1,
                    sample_rate: 16_000,
                })
                .unwrap();
            }
        }
        drop(s_tx);
        drop(m_tx);
        staggered_writer.join().unwrap().unwrap();
        let staggered = load_recording_channels(&staggered_path).unwrap();
        let staggered_sys = staggered.system.unwrap();
        assert_eq!(staggered.mic.len(), 16_000);
        assert_eq!(staggered_sys.len(), 16_000);
        assert!(
            staggered_sys.iter().all(|&s| (s + 0.45).abs() < 1e-3),
            "staggered block arrivals must not insert 0.0 gaps into system audio"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dual_channel_backchannel_over_remote_speech_does_not_fragment_it() {
        // Remote speaker 0 talks 0–20 s in three diarizer segments; "You"
        // says "yeah" at 8.5 s *over* them. A tiny remote piece follows.
        let remote = [(0.0, 4.0, 0), (4.3, 9.0, 0), (9.2, 9.6, 0), (9.8, 20.0, 0)];
        let you = [(8.5, 8.9, YOU_SPEAKER_ID)];
        let turns = merge_dual_channel_turns(&you, &remote);
        let remote_turns: Vec<_> = turns.iter().filter(|t| t.speaker == 0).collect();
        assert_eq!(remote_turns.len(), 1, "{turns:?}");
        assert_eq!((remote_turns[0].start, remote_turns[0].end), (0.0, 20.0));
    }

    #[test]
    fn dual_channel_interjection_in_a_pause_splits_the_remote_turn() {
        // Remote pauses 5.0–6.2 s and "You" answers in the pause: the
        // transcript must read remote, You, remote.
        let remote = [(0.0, 5.0, 0), (6.2, 10.0, 0)];
        let you = [(5.1, 6.0, YOU_SPEAKER_ID)];
        let turns = merge_dual_channel_turns(&you, &remote);
        let order: Vec<i32> = turns.iter().map(|t| t.speaker).collect();
        assert_eq!(order, vec![0, YOU_SPEAKER_ID, 0], "{turns:?}");
    }

    #[test]
    fn stereo_writer_aligns_a_late_starting_system_tap_with_the_mic() {
        // The mic tap attaches 300 ms before the system tap starts delivering
        // (aggregate device creation). A click heard on both at real time
        // 0.5 s must land on the same frame in both channels.
        let dir = std::env::temp_dir().join(format!("mp-meeting-align-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("aligned.wav");
        let (mic_tx, mic_rx) = mpsc::channel();
        let (sys_tx, sys_rx) = mpsc::channel();
        let mut mic = vec![0.0f32; 16_000];
        mic[8_000..8_160].fill(0.9);
        let mut sys = vec![0.0f32; 11_200]; // starts at real time 0.3 s
        sys[3_200..3_360].fill(-0.9);
        for c in mic.chunks(160) {
            mic_tx
                .send(AudioChunk {
                    samples: c.to_vec(),
                    channels: 1,
                    sample_rate: 16_000,
                })
                .unwrap();
        }
        sys_tx
            .send(AudioChunk {
                samples: sys,
                channels: 1,
                sample_rate: 16_000,
            })
            .unwrap();
        drop(mic_tx);
        drop(sys_tx);
        write_meeting_stereo(std::fs::File::create(&path).unwrap(), mic_rx, sys_rx).unwrap();

        let loaded = load_recording_channels(&path).unwrap();
        let sys = loaded.system.unwrap();
        assert_eq!(loaded.mic.len(), 16_000);
        let first = |v: &[f32]| v.iter().position(|x| x.abs() > 0.5).unwrap();
        assert_eq!(first(&loaded.mic), 8_000);
        assert_eq!(first(&sys), 8_000, "system channel shifted against the mic");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn echo_gate_zeroes_speakerphone_bleed_and_preserves_independent_or_double_talk_speech() {
        let n = 16_000 * 3; // 3 seconds at 16 kHz
        let mut system = vec![0.0f32; n];
        let mut mic = vec![0.0f32; n];

        // Segment 1 (0.2s .. 1.0s): Remote speaker plays on system audio; mic picks up
        // delayed (12ms = 192 samples), attenuated (0.35x) speakerphone bleed.
        let lag = 192usize;
        for i in 3200..16_000 {
            let sig = 0.30 * ((i as f32 * 0.11).sin() + 0.5 * (i as f32 * 0.23).cos());
            system[i] = sig;
            if i + lag < n {
                mic[i + lag] = sig * 0.35;
            }
        }

        // Segment 2 (1.5s .. 2.0s): Local user speaks into mic while system is silent.
        for (i, m) in mic.iter_mut().enumerate().take(32_000).skip(24_000) {
            *m = 0.25 * ((i as f32 * 0.17).sin() + 0.4 * (i as f32 * 0.29).cos());
        }

        // Segment 3 (2.2s .. 2.7s): Double-talk — system plays remote speech (0.10),
        // while local user speaks much louder (0.45) with an uncorrelated waveform.
        for i in 35_200..43_200 {
            let sys_sig = 0.10 * (i as f32 * 0.13).sin();
            let user_sig = 0.45 * (i as f32 * 0.31).cos();
            system[i] = sys_sig;
            mic[i] = user_sig + sys_sig * 0.25;
        }

        suppress_speakerphone_echo(&mut mic, &system);

        // Bleed region (0.3s .. 0.9s) must be completely zeroed.
        let bleed_energy: f32 = mic[4800..14_400].iter().map(|x| x * x).sum();
        assert_eq!(
            bleed_energy, 0.0,
            "speakerphone bleed should be zeroed by the echo gate"
        );

        // Solo local speech (1.6s .. 1.9s) must remain intact.
        let solo_energy: f32 = mic[25_600..30_400].iter().map(|x| x * x).sum();
        assert!(
            solo_energy > 50.0,
            "independent local speech must not be suppressed (energy={solo_energy})"
        );

        // Double-talk (2.3s .. 2.6s) must remain intact.
        let dt_energy: f32 = mic[36_800..41_600].iter().map(|x| x * x).sum();
        assert!(
            dt_energy > 100.0,
            "loud local double-talk must not be suppressed (energy={dt_energy})"
        );
    }

    #[test]
    fn group_voices_resists_centroid_snowball_across_six_biased_call_speakers() {
        // Six distinct speakers sharing a strong WebRTC/codec channel component
        // (`alpha^2 = 0.72` cross-speaker cosine, `~0.96` same-speaker cosine).
        // Summing centroids without UPGMA normalization inflates cross-cluster
        // cosine from 0.72 -> 0.79 -> 0.82 -> 0.84 and collapses all 6 into 1.
        let alpha = 0.72f32.sqrt();
        let residual = (1.0 - 0.72f32).sqrt();
        let make_speaker_vec = |spk: usize, jitter: f32| -> Vec<f32> {
            let mut v = vec![0.0f32; 16];
            v[0] = alpha; // shared call-channel direction
            v[1 + spk] = residual;
            v[8 + spk] = jitter;
            v
        };
        let mut voices = Vec::new();
        for round in 0..2 {
            for spk in 0..6 {
                let jitter = if round == 0 { 0.06 } else { -0.06 };
                voices.push(voice(&make_speaker_vec(spk, jitter), 2.5 + round as f32));
            }
        }
        let labels = group_voices(&voices, 0, SAME_SPEAKER_SIMILARITY);
        assert_eq!(
            labels,
            vec![0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5],
            "all 6 biased call speakers must stay distinct across both rounds"
        );
    }

    #[test]
    fn group_voices_complete_linkage_blocks_bridge_chaining() {
        // Speaker A and Speaker C are orthogonal (cosine 0.0), with a blended
        // handoff segment B right between them (~0.707 cosine to both).
        let a = [1.0, 0.0, 0.0];
        let b = [1.0, 1.0, 0.0];
        let c = [0.0, 1.0, 0.0];
        let voices = [voice(&a, 3.0), voice(&b, 2.0), voice(&c, 3.0)];
        let labels = group_voices(&voices, 0, 0.65);
        assert_ne!(
            labels[0], labels[2],
            "complete-linkage guard must prevent bridge segment B from merging A and C: {labels:?}"
        );
    }

    #[test]
    fn group_voices_with_hints_respects_ui_cannot_link_for_close_voices() {
        // Two very similar voices (~0.94 cosine) that would merge acoustically,
        // but Accessibility unambiguously observed two different active tiles.
        let v1 = [1.0, 0.1, 0.0];
        let v2 = [0.98, 0.25, 0.0];
        let voices = [voice(&v1, 3.0), voice(&v2, 3.0)];
        let hints = [Some("Alice".to_string()), Some("Carol".to_string())];
        assert_eq!(
            group_voices_with_hints(&voices, Some(&hints), 0, SAME_SPEAKER_SIMILARITY),
            vec![0, 1]
        );
        // An explicit speaker count override (`speakers = 1`) still merges when requested.
        assert_eq!(
            group_voices_with_hints(&voices, Some(&hints), 1, SAME_SPEAKER_SIMILARITY),
            vec![0, 0]
        );
    }

    #[test]
    fn split_segments_at_ui_changes_cuts_at_quiet_pause_valley() {
        use crate::speaker_poller::SpeakerObservation;
        let mut samples = vec![0.25f32; 16_000 * 6];
        // Quiet 40ms breath pause centered around 2.82s (near the 3.0s UI handoff).
        let valley_start = (2.80 * 16_000.0) as usize;
        let valley_end = (2.84 * 16_000.0) as usize;
        samples[valley_start..valley_end].fill(0.0);

        let observations = vec![
            SpeakerObservation {
                offset_ms: 0,
                speaking_names: vec!["Alice".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 3000,
                speaking_names: vec!["Bob".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 5900,
                speaking_names: vec![],
                muted_names: vec![],
            },
        ];
        let spans = observation_spans(&observations);
        let split = split_segments_at_ui_changes(&samples, &[(0.0, 6.0)], &spans);
        assert_eq!(split.len(), 2, "{split:?}");
        assert!(
            (split[0].1 - 2.82).abs() < 0.03,
            "expected split in the 2.82s pause valley, got {:?}",
            split[0].1
        );
        assert_eq!(split[0].1, split[1].0);

        // Multi-handoff (Alice 0..2.2s -> Bob 2.2..4.4s -> Alice 4.4..7.0s) in a single segment:
        // local windowing around each handoff splits all 3 turns cleanly.
        let samples_multi = vec![0.25f32; 16_000 * 7];
        let multi_obs = vec![
            SpeakerObservation {
                offset_ms: 0,
                speaking_names: vec!["Alice".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 2200,
                speaking_names: vec!["Bob".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 4400,
                speaking_names: vec!["Alice".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 6900,
                speaking_names: vec![],
                muted_names: vec![],
            },
        ];
        let multi_spans = observation_spans(&multi_obs);
        let multi_split = split_segments_at_ui_changes(&samples_multi, &[(0.0, 7.0)], &multi_spans);
        assert_eq!(multi_split.len(), 3, "{multi_split:?}");
    }

    #[test]
    fn resolve_cluster_names_marks_multi_speaker_clump_as_group() {
        use crate::speaker_poller::SpeakerObservation;
        // Cluster 0 has both Alice (4.5s) and Bob (4.5s) clumped together (50% each < 55%);
        // Cluster 1 is unambiguously Carol (5.0s); Cluster 2 has no UI observations.
        let turns = vec![
            Turn {
                start: 0.0,
                end: 4.5,
                speaker: 0,
            },
            Turn {
                start: 5.0,
                end: 9.5,
                speaker: 0,
            },
            Turn {
                start: 10.0,
                end: 15.0,
                speaker: 1,
            },
            Turn {
                start: 16.0,
                end: 18.0,
                speaker: 2,
            },
        ];
        let observations = vec![
            SpeakerObservation {
                offset_ms: 0,
                speaking_names: vec!["Alice".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 4500,
                speaking_names: vec![],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 5000,
                speaking_names: vec!["Bob".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 9500,
                speaking_names: vec![],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 10_000,
                speaking_names: vec!["Carol".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 15_000,
                speaking_names: vec![],
                muted_names: vec![],
            },
        ];
        let names = resolve_cluster_names(&turns, &observations, None);
        assert_eq!(names[&0], "Group");
        assert_eq!(names[&1], "Carol");
        assert_eq!(names[&2], "Speaker 1");
    }

    #[test]
    fn remote_group_fallback_and_meeting_detail_handle_clumped_vs_single_voice() {
        use crate::speaker_poller::SpeakerObservation;
        let turns = vec![
            Turn {
                start: 0.0,
                end: 2.0,
                speaker: YOU_SPEAKER_ID,
            },
            Turn {
                start: 3.0,
                end: 12.0,
                speaker: 0,
            },
        ];

        // 1. Unconfirmed single remote bucket with `speakers = 0` -> "Group"
        let mut names = resolve_cluster_names(&turns, &[], Some(YOU_SPEAKER_ID));
        apply_remote_group_fallback(&mut names, &turns, &[], 0, false);
        assert_eq!(names[&YOU_SPEAKER_ID], "You");
        assert_eq!(names[&0], "Group");

        let lines = vec![
            Line {
                start: 0.0,
                speaker: "You".into(),
                text: "Quick stand-up?".into(),
            },
            Line {
                start: 3.0,
                speaker: "Group".into(),
                text: "All good on our end.".into(),
            },
        ];
        assert_eq!(meeting_detail(600.0, &lines), "10 min · You + group");
        assert_eq!(meeting_detail(600.0, &lines[1..]), "10 min · group");
        let mixed_lines = vec![
            lines[0].clone(),
            Line {
                start: 2.5,
                speaker: "Alice".into(),
                text: "Shipped auth.".into(),
            },
            lines[1].clone(),
        ];
        assert_eq!(meeting_detail(600.0, &mixed_lines), "10 min · 3+ speakers");

        // 2. Confirmed single voice on a 1:1 call (`confirmed_single_voice = true`, <=1 UI speaker)
        //    -> stays "Speaker 1"
        let mut one_on_one = resolve_cluster_names(&turns, &[], Some(YOU_SPEAKER_ID));
        apply_remote_group_fallback(&mut one_on_one, &turns, &[], 0, true);
        assert_eq!(one_on_one[&0], "Speaker 1");

        // 3. Even if `confirmed_single_voice = true`, UI seeing >= 2 active remote names -> "Group"
        let multi_ui = vec![
            SpeakerObservation {
                offset_ms: 3100,
                speaking_names: vec!["Alice".into()],
                muted_names: vec![],
            },
            SpeakerObservation {
                offset_ms: 4000,
                speaking_names: vec!["Bob".into()],
                muted_names: vec![],
            },
        ];
        let mut ui_override = resolve_cluster_names(&turns, &multi_ui, Some(YOU_SPEAKER_ID));
        apply_remote_group_fallback(&mut ui_override, &turns, &multi_ui, 0, true);
        assert_eq!(ui_override[&0], "Group");

        // 4. Explicit `speakers = 2` (user configured a 2-person call) -> keeps "Speaker 1"
        let mut explicit_two = resolve_cluster_names(&turns, &[], Some(YOU_SPEAKER_ID));
        apply_remote_group_fallback(&mut explicit_two, &turns, &[], 2, false);
        assert_eq!(explicit_two[&0], "Speaker 1");

        // 5. Direct check of `single_voice_consistent_from_embeddings`
        let tight = [voice(&[1.0, 0.0, 0.0], 2.5), voice(&[0.98, 0.15, 0.0], 2.5)];
        assert!(single_voice_consistent_from_embeddings(&tight, None));
        let loose = [voice(&[1.0, 0.0, 0.0], 2.5), voice(&[0.70, 0.71, 0.0], 2.5)];
        assert!(!single_voice_consistent_from_embeddings(&loose, None));
        let short_only = [voice(&[1.0, 0.0, 0.0], 0.7), voice(&[1.0, 0.0, 0.0], 0.8)];
        assert!(!single_voice_consistent_from_embeddings(&short_only, None));
        let single_long = [voice(&[1.0, 0.0, 0.0], 5.0)];
        assert!(single_voice_consistent_from_embeddings(
            &single_long,
            Some(0.86)
        ));
        assert!(!single_voice_consistent_from_embeddings(
            &single_long,
            Some(0.65)
        ));
    }

    #[test]
    fn render_markdown_cleans_call_app_speaker_names() {
        let lines = vec![
            Line {
                start: 0.0,
                speaker: "**Dana\n(Host)\u{7}".into(),
                text: "Welcome.".into(),
            },
            Line {
                start: 4.0,
                speaker: "You".into(),
                text: "Thanks.".into(),
            },
        ];
        let md = render_markdown("Call", "1 min · 2 speakers", &lines);
        assert!(md.contains("**Dana(Host)** · 00:00\nWelcome."), "{md}");
        let doc = parse_meeting_markdown(&md);
        assert_eq!(doc.turns.len(), 2);
        assert_eq!(doc.turns[0].speaker, "Dana(Host)");
    }

    #[test]
    fn write_private_replaces_the_file_whole_and_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mp-meeting-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-10-26-174005.md");
        std::fs::write(&path, "old transcript, much longer than the new one").unwrap();
        write_private(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // No temp file left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_and_rename_meeting_markdown_roundtrips_notes_and_merges_turns() {
        let lines = vec![
            Line {
                start: 0.0,
                speaker: "You".into(),
                text: "Good morning.".into(),
            },
            Line {
                start: 5.0,
                speaker: "Speaker 1".into(),
                text: "Shipped the API.".into(),
            },
            Line {
                start: 12.0,
                speaker: "Speaker 2".into(),
                text: "Also fixed the tests.".into(),
            },
        ];
        let md = render_markdown("Morning Stand-up", "15 min · 3 speakers", &lines);
        let mut doc = parse_meeting_markdown(&md);
        assert_eq!(doc.title, "Morning Stand-up");
        assert_eq!(doc.detail, "15 min · 3 speakers");
        assert_eq!(doc.notes, None);
        assert_eq!(doc.turns.len(), 3);
        assert_eq!(doc.to_markdown(), md);

        // Attach notes and rename Speaker 1 -> Alex.
        doc.notes = Some("- **Summary**: Stand-up updates.\n- **Action items**: Deploy.".into());
        assert!(doc.rename_speaker("Speaker 1", "Alex"));
        assert_eq!(doc.turns[1].speaker, "Alex");
        assert_eq!(doc.detail, "15 min · 3 speakers");

        // Rename Speaker 2 -> Alex as well; adjacent Alex turns merge and detail updates to 2 speakers.
        assert!(doc.rename_speaker("Speaker 2", "Alex"));
        assert_eq!(doc.turns.len(), 2);
        assert_eq!(doc.turns[1].speaker, "Alex");
        assert_eq!(doc.turns[1].text, "Shipped the API. Also fixed the tests.");
        assert_eq!(doc.detail, "15 min · 2 speakers");

        let rendered = doc.to_markdown();
        let reparsed = parse_meeting_markdown(&rendered);
        assert_eq!(reparsed, doc);

        // Freeform notes before the first turn (without `## Notes`) and bold text in a turn body
        // are preserved without being mistaken for turn headers or dropped.
        let freeform = "# Stand-up\n\n10 min · 2 speakers\n\nManual context note.\n\n**You** · 00:00\n**Note** · check status.\n";
        let parsed_freeform = parse_meeting_markdown(freeform);
        assert_eq!(
            parsed_freeform.notes.as_deref(),
            Some("Manual context note.")
        );
        assert_eq!(parsed_freeform.turns.len(), 1);
        assert_eq!(parsed_freeform.turns[0].text, "**Note** · check status.");
    }
}
