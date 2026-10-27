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
/// A forgotten meeting stops itself rather than filling the disk (about
/// 115 MB of audio per hour).
const MAX_DURATION: Duration = Duration::from_secs(4 * 60 * 60);
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
}

struct Recording {
    id: String,
    started_ms: u64,
    writer: std::thread::JoinHandle<Result<(), String>>,
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

pub fn status(app: &AppHandle) -> MeetingStatus {
    let state = app.state::<MeetingState>();
    let recording = lock(&state.recording).as_ref().map(|r| RecordingStatus {
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

    let (tx, rx) = mpsc::channel();
    let writer = std::thread::spawn(move || write_meeting(file, rx));
    recording_state.audio.set_meeting_tap(Some(tx));
    *lock(&app.state::<MeetingState>().recording) = Some(Recording {
        id: id.clone(),
        started_ms: now_ms(),
        writer,
    });
    crate::applog!("meeting: recording {id}");
    crate::refresh_tray_indicator(app);
    let _ = app.emit("recording-started", ());
    announce(app);

    // The widget's level meter, and the safety stop.
    let level_app = app.clone();
    std::thread::spawn(move || {
        let state = level_app.state::<RecordingState>();
        let started = Instant::now();
        while state.capture.is_session_active(session) {
            let _ = level_app.emit("audio-level", state.audio.current_level());
            if started.elapsed() >= MAX_DURATION {
                crate::applog!(
                    "meeting: stopping after {} hours",
                    MAX_DURATION.as_secs() / 3600
                );
                let _ = stop(&level_app);
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
    let Some(recording) = recording else {
        announce(app);
        return Ok(());
    };
    let id = recording.id.clone();
    crate::applog!("meeting: stopped {id}");
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        transcribe_file(app, id)
    }))
    .unwrap_or_else(|_| {
        Err(
            "Something went wrong while transcribing. The recording is kept; try Transcribe again."
                .into(),
        )
    });
    match result {
        Ok(summary) => {
            crate::applog!(
                "meeting: transcribed {id} in {:.0}s ({})",
                started.elapsed().as_secs_f32(),
                summary.detail.as_deref().unwrap_or("")
            );
            let _ = app.emit("meeting-saved", summary.title);
        }
        Err(err) => {
            crate::applog!("meeting: couldn't transcribe {id}: {err}");
            let _ = app.emit("meeting-failed", err);
        }
    }
}

fn transcribe_file(app: &AppHandle, id: &str) -> Result<MeetingSummary, String> {
    let dir = meetings_dir(app)?;
    let wav = dir.join(format!("{id}.wav"));
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
    let samples = load_recording(&wav)?;
    let duration = samples.len() as f32 / WHISPER_SAMPLE_RATE as f32;
    if duration < MIN_SECONDS {
        let _ = std::fs::remove_file(&wav);
        return Err("The meeting was too short to transcribe.".into());
    }

    let speakers = crate::config::load(app).meeting_speakers;
    let models = DiarizationModels {
        segmentation: resource_path(app, SEGMENTATION_MODEL)?,
        embedding: crate::voice_isolation::resolve_model_path(app)?,
    };
    let turns = diarize(&samples, &models, speakers)?;
    if turns.is_empty() {
        if crate::audio::is_dead_air(&samples) {
            let _ = std::fs::remove_file(&wav);
            return Err("The meeting recording was silent. Check the microphone.".into());
        }
        return Err("No speech was found in the meeting recording.".into());
    }

    let you = find_you(app, &samples, &turns);
    let names = speaker_names(&turns, you);
    let whisper = &app.state::<RecordingState>().whisper;
    let mut lines = Vec::new();
    let total = turns.len();
    for (index, turn) in turns.iter().enumerate() {
        progress("Transcribing", Some((index * 100 / total).min(99) as u8));
        let clip = clip(&samples, turn);
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
                speaker: names[&turn.speaker].clone(),
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
    Ok(MeetingSummary {
        id: id.to_string(),
        title,
        detail: Some(detail),
        state: MeetingFileState::Ready,
    })
}

/// The recording as samples. It's always 16 kHz mono 16-bit, read straight
/// into one buffer (a 4-hour meeting is ~900 MB as f32; the general loader
/// would hold three copies). A crash can leave a partial last sample,
/// which is skipped.
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

/// A clip cut into roughly equal pieces of at most `MAX_PIECE_SECONDS`.
fn pieces(clip: &[f32]) -> std::slice::Chunks<'_, f32> {
    let max = (MAX_PIECE_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
    let count = clip.len().div_ceil(max).max(1);
    clip.chunks(clip.len().div_ceil(count).max(1))
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())
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
/// shapes the segment boundaries), then each whole segment gets one voice
/// embedding and `group_voices` decides who is who.
pub fn diarize(
    samples: &[f32],
    models: &DiarizationModels,
    speakers: u32,
) -> Result<Vec<Turn>, String> {
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
    let segments: Vec<(f32, f32)> = result
        .sort_by_start_time()
        .into_iter()
        .map(|s| (s.start, s.end))
        .filter(|(start, end)| end - start >= MIN_SEGMENT_SECONDS)
        .collect();
    drop(diarizer);

    let extractor = SpeakerEmbeddingExtractor::create(&embedding)
        .ok_or("couldn't load the speaker embedding model")?;
    let rate = WHISPER_SAMPLE_RATE as f32;
    let voices: Vec<Voice> = segments
        .iter()
        .map(|&(start, end)| {
            let a = ((start * rate) as usize).min(samples.len());
            let b = ((end * rate) as usize).clamp(a, samples.len());
            let stream = extractor.create_stream()?;
            stream.accept_waveform(WHISPER_SAMPLE_RATE as i32, &samples[a..b]);
            stream.input_finished();
            extractor.is_ready(&stream).then_some(())?;
            extractor.compute(&stream)
        })
        .zip(&segments)
        .map(|(embedding, &(start, end))| Voice {
            embedding,
            seconds: end - start,
        })
        .collect();
    let labels = group_voices(&voices, speakers, SAME_SPEAKER_SIMILARITY);
    let labelled: Vec<(f32, f32, i32)> = segments
        .iter()
        .zip(labels)
        .map(|(&(start, end), label)| (start, end, label as i32))
        .collect();
    Ok(merge_turns(&labelled))
}

/// sherpa-onnx's grouping threshold, set low so that it splits rather than
/// merges; `group_voices` does the real grouping.
const OVERSPLIT_THRESHOLD: f32 = 0.15;
const MIN_SEGMENT_SECONDS: f32 = 0.3;
/// Segments at least this long set the speakers; shorter ones (noisier
/// embeddings) then join whichever speaker they sound most like.
const ANCHOR_SECONDS: f32 = 1.0;
/// Average-voice similarity above which two groups are taken to be the
/// same person when the speaker count isn't given. One person across
/// utterances scores about 0.85-0.9 with this model; different people
/// mostly score well under 0.6.
const SAME_SPEAKER_SIMILARITY: f32 = 0.6;
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
/// Agglomerative: every long segment starts as its own group, and the two
/// most alike groups (by their duration-weighted average voice) merge
/// until there are `speakers` groups, or, with `speakers` 0, until no two
/// are more alike than `same_speaker`. Short segments then join the group
/// they're closest to; a segment with no embedding takes its neighbour's.
pub fn group_voices(voices: &[Voice], speakers: u32, same_speaker: f32) -> Vec<usize> {
    let mut anchors: Vec<usize> = (0..voices.len())
        .filter(|&i| voices[i].embedding.is_some() && voices[i].seconds >= ANCHOR_SECONDS)
        .collect();
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

    // (sum of weighted unit vectors, members); `None` once merged away.
    let mut groups: Vec<Option<(Vec<f32>, Vec<usize>)>> = anchors
        .iter()
        .map(|&i| {
            let v = &voices[i];
            let unit = normalized(v.embedding.as_ref().unwrap());
            Some((unit.iter().map(|x| x * v.seconds).collect(), vec![i]))
        })
        .collect();
    // Pairwise similarity, updated only for the merged group each step.
    let n = groups.len();
    let mut similarity = vec![vec![f32::NEG_INFINITY; n]; n];
    for a in 0..n {
        for b in a + 1..n {
            let s = cosine(
                &groups[a].as_ref().unwrap().0,
                &groups[b].as_ref().unwrap().0,
            );
            similarity[a][b] = s;
            similarity[b][a] = s;
        }
    }
    let mut live = n;
    let target = speakers.max(1) as usize;
    while live > 1 && !(speakers > 0 && live <= target) {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..n {
            if groups[a].is_none() {
                continue;
            }
            for b in a + 1..n {
                if groups[b].is_some() && best.is_none_or(|(_, _, s)| similarity[a][b] > s) {
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
        let (sum, members) = groups[b].take().unwrap();
        let merged = groups[a].as_mut().unwrap();
        for (x, y) in merged.0.iter_mut().zip(sum) {
            *x += y;
        }
        merged.1.extend(members);
        live -= 1;
        for other in 0..n {
            if other != a {
                if let Some((sum, _)) = &groups[other] {
                    let s = cosine(&groups[a].as_ref().unwrap().0, sum);
                    similarity[a][other] = s;
                    similarity[other][a] = s;
                }
            }
        }
    }
    let mut groups: Vec<(Vec<f32>, Vec<usize>)> = groups.into_iter().flatten().collect();

    // Number groups by first appearance.
    groups.sort_by_key(|(_, members)| members.iter().copied().min());
    let mut labels: Vec<Option<usize>> = vec![None; voices.len()];
    for (label, (_, members)) in groups.iter().enumerate() {
        for &i in members {
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
                        cosine(&x.1 .0, embedding).total_cmp(&cosine(&y.1 .0, embedding))
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
    let mut speakers: Vec<&str> = lines.iter().map(|l| l.speaker.as_str()).collect();
    speakers.sort_unstable();
    speakers.dedup();
    let people = match speakers.len() {
        1 => "1 speaker".to_string(),
        n => format!("{n} speakers"),
    };
    format!("{minutes} min · {people}")
}

/// The transcript: one paragraph per turn, with consecutive turns from the
/// same speaker joined.
pub fn render_markdown(title: &str, detail: &str, lines: &[Line]) -> String {
    let mut out = format!("# {title}\n\n{detail}");
    let mut previous: Option<&str> = None;
    for line in lines {
        if previous == Some(line.speaker.as_str()) {
            out.push(' ');
            out.push_str(&line.text);
        } else {
            out.push_str(&format!(
                "\n\n**{}** · {}\n{}",
                line.speaker,
                timestamp(line.start),
                line.text
            ));
            previous = Some(line.speaker.as_str());
        }
    }
    out.push('\n');
    out
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
    let detail = lines.next();
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
    for ext in ["md", "wav"] {
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
    cfg.meeting_speakers = speakers.min(10);
    crate::config::save(&app, &cfg)
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
}
