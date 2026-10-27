use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

enum AudioCommand {
    Start,
    Stop(Sender<Result<PathBuf, String>>),
    /// Attach (`Some`) or detach (`None`) the ambient listener. While
    /// attached the stream keeps running between recordings.
    SetTap(Option<Sender<AudioChunk>>),
    /// Attach or detach the meeting recorder, which (like the ambient tap)
    /// keeps the stream running while attached.
    SetMeetingTap(Option<Sender<AudioChunk>>),
    /// The selected device changed: open it now (paused, unless something
    /// is listening) so its status is known immediately, not at the next
    /// recording.
    Refresh,
}

/// Handle stored in Tauri state. The `cpal::Stream` itself is not `Send`/`Sync`
/// on every platform, so it lives on a dedicated thread and is driven by
/// commands sent over this channel instead.
pub struct AudioHandle {
    tx: Sender<AudioCommand>,
    /// `None` means "use the system default input device". Shared with the
    /// audio thread so a device picked in Settings takes effect on the next
    /// recording without needing a dedicated command/round-trip.
    selected_device: Arc<Mutex<Option<String>>>,
    /// Most recent input level (0.0-1.0, see `rms_level`), bit-packed since
    /// there's no stable `AtomicF32`. Updated on every audio callback,
    /// polled by `recording.rs`'s level-meter thread at a much lower,
    /// UI-appropriate rate — decoupled from the callback's own cadence
    /// rather than emitting an event per callback, which would be far more
    /// often than any redraw needs.
    level: Arc<AtomicU32>,
    status: Arc<Mutex<Option<DeviceStatus>>>,
    /// The push-to-talk buffer and its format, shared so the live
    /// transcriber can read what's been recorded so far (`recorded_since`).
    samples: Arc<Mutex<Vec<f32>>>,
    format: Arc<AtomicU64>,
    /// Bumped by each `start`, so a reader from an earlier recording can
    /// tell the buffer now holds a different one.
    generation: AtomicU64,
}

/// Which microphone recording will actually use, as surfaced to the widget.
/// `Fallback` is the case that used to be silent: the picked mic wasn't
/// available, so capture quietly switched to the system default.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceStatus {
    Preferred { using: String },
    Fallback { wanted: String, using: String },
    SystemDefault { using: String },
    Unavailable { wanted: Option<String> },
}

impl AudioHandle {
    /// `initial_device` is passed in rather than set afterwards so the
    /// warm stream built here is for the right device; setting it after
    /// spawn raced the thread and could force a rebuild (with retry
    /// sleeps) onto the first hotkey press.
    pub fn spawn(
        initial_device: Option<String>,
        on_status: impl Fn(&DeviceStatus) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<AudioCommand>();
        let selected_device: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(initial_device));
        let selected_device_thread = selected_device.clone();
        let level: Arc<AtomicU32> = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let level_thread = level.clone();
        let status: Arc<Mutex<Option<DeviceStatus>>> = Arc::new(Mutex::new(None));
        let status_thread = status.clone();
        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let format = Arc::new(AtomicU64::new(0));
        let (samples_thread, format_thread) = (samples.clone(), format.clone());

        std::thread::spawn(move || {
            let sinks = Sinks {
                samples: samples_thread,
                format: format_thread,
                buffering: Arc::new(AtomicBool::new(false)),
                tap: Arc::new(Mutex::new(None)),
                meeting: Arc::new(Mutex::new(None)),
                level: level_thread.clone(),
            };
            let publish = |stream: &Option<WarmStream>, wanted: &Option<String>| {
                let next = device_status(
                    stream.as_ref().map(WarmStream::facts).as_ref(),
                    wanted.as_deref(),
                );
                let mut last = status_thread.lock().unwrap();
                if last.as_ref() != Some(&next) {
                    crate::applog!("audio: device status -> {next:?}");
                    on_status(&next);
                    *last = Some(next);
                }
            };

            // The stream is built once and kept around *paused* between
            // recordings rather than torn down after every `Stop` — device
            // enumeration + format negotiation (the expensive part) happens
            // ahead of time instead of on the hotkey's critical path, so
            // speech in the first moments after the hotkey isn't lost
            // waiting for the driver to spin up. It's only rebuilt when
            // `rebuild_reason` says it's no longer the right stream.

            // Makes sure `current` is the right stream and playing.
            let ensure_playing = |current: &mut Option<WarmStream>| {
                let device_name = selected_device_thread.lock().unwrap().clone();
                let facts = current.as_ref().map(WarmStream::facts);
                if let Some(reason) = rebuild_reason(
                    facts.as_ref(),
                    device_name.as_deref(),
                    |name| list_device_names().iter().any(|d| d == name),
                    default_device_name,
                ) {
                    crate::applog!("audio: rebuilding stream ({reason})");
                    *current = build_warm_stream(&sinks, device_name.clone());
                }

                if current
                    .as_ref()
                    .and_then(|cs| cs.stream.play().ok())
                    .is_none()
                {
                    crate::applog!("audio: resuming warm stream failed, rebuilding fresh");
                    *current = build_warm_stream(&sinks, device_name.clone());
                    match current.as_ref().map(|cs| cs.stream.play()) {
                        Some(Ok(())) => crate::applog!("audio: stream playing (rebuilt)"),
                        Some(Err(err)) => {
                            crate::applog!("audio: stream.play() failed after rebuild: {err}")
                        }
                        None => crate::applog!("audio: failed to start capture"),
                    }
                }
                publish(current, &device_name);
            };

            let pause_if_unused = |current: &Option<WarmStream>| {
                if sinks.buffering.load(Ordering::SeqCst) || sinks.tapped() {
                    return;
                }
                if let Some(cs) = current {
                    if let Err(err) = cs.stream.pause() {
                        crate::applog!("audio: stream.pause() failed: {err}");
                    }
                }
                // Otherwise the widget's level meter would hold whatever
                // the last in-flight callback reported.
                level_thread.store(0.0f32.to_bits(), Ordering::Relaxed);
            };

            // cpal starts a stream as soon as it's built, so pause right away:
            // otherwise the mic is live (and the macOS mic indicator lit)
            // from launch, with nobody listening.
            let initial = selected_device_thread.lock().unwrap().clone();
            let mut current = build_warm_stream(&sinks, initial.clone());
            pause_if_unused(&current);
            publish(&current, &initial);

            for command in rx {
                match command {
                    AudioCommand::Start => {
                        sinks.samples.lock().unwrap().clear();
                        ensure_playing(&mut current);
                        sinks.buffering.store(true, Ordering::SeqCst);
                        crate::applog!("audio: recording");
                    }
                    AudioCommand::Stop(reply) => {
                        sinks.buffering.store(false, Ordering::SeqCst);
                        pause_if_unused(&current);
                        let captured = std::mem::take(&mut *sinks.samples.lock().unwrap());
                        crate::applog!("audio: stopped, captured {} samples", captured.len());
                        let result = match &current {
                            Some(cs) if !captured.is_empty() => {
                                write_wav(&captured, cs.sample_rate, cs.channels)
                            }
                            _ => Err("no audio captured".to_string()),
                        };
                        let _ = reply.send(result);
                    }
                    AudioCommand::Refresh => {
                        // Mid-recording, swapping streams would splice two
                        // devices (maybe at different sample rates) into one
                        // clip; the next Start picks the new device up.
                        if sinks.buffering.load(Ordering::SeqCst) {
                            continue;
                        }
                        if sinks.tapped() {
                            ensure_playing(&mut current);
                        } else {
                            let device_name = selected_device_thread.lock().unwrap().clone();
                            let facts = current.as_ref().map(WarmStream::facts);
                            if let Some(reason) = rebuild_reason(
                                facts.as_ref(),
                                device_name.as_deref(),
                                |name| list_device_names().iter().any(|d| d == name),
                                default_device_name,
                            ) {
                                crate::applog!("audio: rebuilding stream ({reason})");
                                current = build_warm_stream(&sinks, device_name.clone());
                            }
                            pause_if_unused(&current);
                            publish(&current, &device_name);
                        }
                    }
                    AudioCommand::SetTap(tap) => {
                        let attach = tap.is_some();
                        *sinks.tap.lock().unwrap() = tap;
                        if attach {
                            ensure_playing(&mut current);
                            crate::applog!("audio: ambient tap attached");
                        } else {
                            pause_if_unused(&current);
                            crate::applog!("audio: ambient tap detached");
                        }
                    }
                    AudioCommand::SetMeetingTap(tap) => {
                        let attach = tap.is_some();
                        *sinks.meeting.lock().unwrap() = tap;
                        if attach {
                            ensure_playing(&mut current);
                            crate::applog!("audio: meeting recorder attached");
                        } else {
                            pause_if_unused(&current);
                            crate::applog!("audio: meeting recorder detached");
                        }
                    }
                }
            }
        });

        Self {
            tx,
            selected_device,
            level,
            status,
            samples,
            format,
            generation: AtomicU64::new(0),
        }
    }

    pub fn start(&self) {
        // Cleared here as well as on the audio thread, so nothing from the
        // last recording is readable under the new generation. Callbacks
        // don't append until the thread handles `Start`.
        {
            let mut samples = self.samples.lock().unwrap();
            samples.clear();
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        let _ = self.tx.send(AudioCommand::Start);
    }

    /// Which recording the buffer holds; see `recorded_since`.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Route every captured chunk to `tap` (ambient listening) until called
    /// again with `None`.
    pub fn set_tap(&self, tap: Option<Sender<AudioChunk>>) {
        let _ = self.tx.send(AudioCommand::SetTap(tap));
    }

    /// Route every captured chunk to the meeting recorder until called
    /// again with `None`.
    pub fn set_meeting_tap(&self, tap: Option<Sender<AudioChunk>>) {
        let _ = self.tx.send(AudioCommand::SetMeetingTap(tap));
    }

    pub fn stop(&self) -> Result<PathBuf, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(AudioCommand::Stop(reply_tx))
            .map_err(|_| "audio thread unavailable".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "audio thread did not respond".to_string())?
    }

    /// Everything captured since `offset` (counted in raw interleaved
    /// samples) in the current push-to-talk recording, with the stream's
    /// channel count and sample rate. Empty (and rate 0) before the first
    /// audio callback. An `offset` past the end means the buffer was
    /// restarted, so it reads from the beginning. Returns nothing once a
    /// newer recording than `generation` has started.
    pub fn recorded_since(&self, offset: usize, generation: u64) -> (Vec<f32>, u16, u32) {
        // Format is read under the lock: a callback stores it before it
        // appends, so any samples seen here come with their own format.
        let samples = self.samples.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != generation {
            return (Vec::new(), 0, 0);
        }
        let (rate, channels) = unpack_format(self.format.load(Ordering::Relaxed));
        let from = if offset > samples.len() { 0 } else { offset };
        (samples[from..].to_vec(), channels, rate)
    }

    /// `None` resets to the system default input device.
    pub fn set_device(&self, name: Option<String>) {
        *self.selected_device.lock().unwrap() = name;
        let _ = self.tx.send(AudioCommand::Refresh);
    }

    /// The explicitly selected device, if any (does not resolve the default).
    pub fn selected_device(&self) -> Option<String> {
        self.selected_device.lock().unwrap().clone()
    }

    /// Last published device status (`None` before the first stream build).
    pub fn device_status(&self) -> Option<DeviceStatus> {
        self.status.lock().unwrap().clone()
    }

    /// Most recent input level, 0.0-1.0 — 0.0 at rest (not recording, or no
    /// callback has landed yet). Polled by `recording.rs`'s level-meter
    /// thread; not itself pushed anywhere, so reading it costs nothing when
    /// nobody's watching.
    pub fn current_level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

fn pack_format(sample_rate: u32, channels: u16) -> u64 {
    (sample_rate as u64) << 16 | channels as u64
}

fn unpack_format(packed: u64) -> (u32, u16) {
    ((packed >> 16) as u32, (packed & 0xffff) as u16)
}

/// One callback's worth of captured audio, for the ambient listener and
/// the meeting recorder.
pub struct AudioChunk {
    /// Interleaved, `channels` per frame, at `sample_rate`.
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Where each audio callback's samples go. Cloned into every stream built,
/// so a rebuilt stream (device change) keeps feeding the same consumers.
#[derive(Clone)]
struct Sinks {
    /// The push-to-talk recording buffer.
    samples: Arc<Mutex<Vec<f32>>>,
    /// Only while true do callbacks append to `samples` — the stream also
    /// runs for ambient listening, and idle audio mustn't accumulate there.
    buffering: Arc<AtomicBool>,
    /// Sample rate and channel count of the latest callback, packed by
    /// `pack_format`.
    format: Arc<AtomicU64>,
    /// The ambient listener, if it's running.
    tap: Arc<Mutex<Option<Sender<AudioChunk>>>>,
    /// The meeting recorder, while a meeting is being recorded.
    meeting: Arc<Mutex<Option<Sender<AudioChunk>>>>,
    level: Arc<AtomicU32>,
}

impl Sinks {
    fn deliver(&self, data: &[f32], channels: u16, sample_rate: u32) {
        self.level
            .store(rms_level(data).to_bits(), Ordering::Relaxed);
        if self.buffering.load(Ordering::Relaxed) {
            self.format
                .store(pack_format(sample_rate, channels), Ordering::Relaxed);
            self.samples.lock().unwrap().extend_from_slice(data);
        }
        for tap in [&self.tap, &self.meeting] {
            if let Some(tap) = tap.lock().unwrap().as_ref() {
                let _ = tap.send(AudioChunk {
                    samples: data.to_vec(),
                    channels,
                    sample_rate,
                });
            }
        }
    }

    /// Whether something besides push-to-talk needs the stream running.
    fn tapped(&self) -> bool {
        self.tap.lock().unwrap().is_some() || self.meeting.lock().unwrap().is_some()
    }
}

/// A built (but not necessarily playing) input stream, kept alive between
/// recordings so `Start` only has to call `play()` instead of paying the
/// full device-open cost from cold.
struct WarmStream {
    stream: cpal::Stream,
    /// What the caller asked for (`None` = system default).
    requested_device: Option<String>,
    /// What actually opened.
    resolved_name: String,
    /// The requested device was unavailable and this is the system default
    /// standing in for it.
    fallback: bool,
    /// Set by the stream's error callback (e.g. the device was unplugged
    /// while idle). A dead stream can still report `play()` as Ok and then
    /// deliver nothing, so it has to be rebuilt rather than resumed.
    invalid: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
}

/// The parts of a `WarmStream` the rebuild decision needs, split out so the
/// decision is testable without real audio hardware.
#[derive(Clone, Debug)]
struct StreamFacts {
    requested: Option<String>,
    resolved: String,
    fallback: bool,
    invalid: bool,
}

impl WarmStream {
    fn facts(&self) -> StreamFacts {
        StreamFacts {
            requested: self.requested_device.clone(),
            resolved: self.resolved_name.clone(),
            fallback: self.fallback,
            invalid: self.invalid.load(Ordering::SeqCst),
        }
    }
}

/// Why the warm stream can't be reused for a recording of `requested`, or
/// `None` if it can. The probes are only called when relevant, since device
/// enumeration isn't free.
fn rebuild_reason(
    facts: Option<&StreamFacts>,
    requested: Option<&str>,
    preferred_present: impl FnOnce(&str) -> bool,
    default_now: impl FnOnce() -> Option<String>,
) -> Option<&'static str> {
    let Some(facts) = facts else {
        return Some("no stream yet");
    };
    if facts.invalid {
        return Some("stream reported an error");
    }
    if facts.requested.as_deref() != requested {
        return Some("selected device changed");
    }
    match requested {
        Some(name) if facts.fallback && preferred_present(name) => Some("preferred device is back"),
        None if default_now().as_deref() != Some(facts.resolved.as_str()) => {
            Some("system default device changed")
        }
        _ => None,
    }
}

fn device_status(stream: Option<&StreamFacts>, wanted: Option<&str>) -> DeviceStatus {
    match (stream, wanted) {
        (None, wanted) => DeviceStatus::Unavailable {
            wanted: wanted.map(str::to_string),
        },
        // A fallback that landed on the wanted device anyway (it wasn't
        // enumerated yet at launch, but is the system default) isn't a
        // fallback from the user's point of view.
        (Some(s), Some(wanted)) if s.fallback && s.resolved != wanted => DeviceStatus::Fallback {
            wanted: wanted.to_string(),
            using: s.resolved.clone(),
        },
        (Some(s), Some(_)) => DeviceStatus::Preferred {
            using: s.resolved.clone(),
        },
        (Some(s), None) => DeviceStatus::SystemDefault {
            using: s.resolved.clone(),
        },
    }
}

/// Builds a stream for `device_name` (logging the resolved device/rate/
/// channels), leaving it paused/not-yet-playing. `None` on failure — the
/// caller treats capture as unavailable.
fn build_warm_stream(sinks: &Sinks, device_name: Option<String>) -> Option<WarmStream> {
    match open_with_retry_and_fallback(sinks, device_name.as_deref()) {
        Ok(opened) => {
            crate::applog!(
                "audio: opened device '{}' at {}Hz, {}ch{}",
                opened.resolved_name,
                opened.sample_rate,
                opened.channels,
                if opened.fallback { " (fallback)" } else { "" }
            );
            Some(WarmStream {
                stream: opened.stream,
                requested_device: device_name,
                resolved_name: opened.resolved_name,
                fallback: opened.fallback,
                invalid: opened.invalid,
                sample_rate: opened.sample_rate,
                channels: opened.channels,
            })
        }
        Err(err) => {
            crate::applog!("audio: failed to open device: {err}");
            None
        }
    }
}

/// Input quieter than this (on the widget meter's 0–1 scale) is treated as
/// "nothing coming through": about -84 dBFS, below the self-noise of any
/// working mic in a quiet room. A muted or disconnected device delivers
/// exact zeros, and a device that never starts delivering stays at 0.
const DEAD_LEVEL: f32 = 0.03;
const DEAD_FOR: std::time::Duration = std::time::Duration::from_secs(2);

/// Watches the live input level during one recording and says when to
/// raise (`Some(true)`) or clear (`Some(false)`) the "no sound from your
/// mic" warning.
#[derive(Default)]
pub struct SilenceWatch {
    quiet_since: Option<std::time::Instant>,
    warned: bool,
}

impl SilenceWatch {
    pub fn observe(&mut self, level: f32, now: std::time::Instant) -> Option<bool> {
        if level >= DEAD_LEVEL {
            self.quiet_since = None;
            return std::mem::take(&mut self.warned).then_some(false);
        }
        let since = *self.quiet_since.get_or_insert(now);
        if !self.warned && now.duration_since(since) >= DEAD_FOR {
            self.warned = true;
            return Some(true);
        }
        None
    }
}

/// A finished recording with essentially no signal (RMS ≈ the same -84
/// dBFS floor): the mic delivered nothing, as opposed to you not speaking.
pub fn is_dead_air(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return true;
    }
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    rms < 6e-5
}

/// Names of all available input devices, for the settings picker.
pub fn list_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// The name of whichever device cpal would pick with no explicit selection.
pub fn default_device_name() -> Option<String> {
    cpal::default_host()
        .default_input_device()
        .and_then(|d| d.name().ok())
}

struct OpenedStream {
    stream: cpal::Stream,
    sample_rate: u32,
    channels: u16,
    resolved_name: String,
    fallback: bool,
    invalid: Arc<AtomicBool>,
}

/// Continuity/Bluetooth input devices (e.g. "iPhone Microphone") can report
/// a stale handle right after being selected — the underlying CoreAudio
/// object gets renegotiated and a snapshot `Device` from a moment earlier
/// no longer resolves. Retrying with a fresh enumeration usually recovers
/// within a second. A device that isn't enumerated at all (unplugged,
/// headset off) skips the retries, since sleeping won't bring it back and
/// they'd only delay recording. Either way, fall back to the system default
/// rather than silently capturing nothing.
fn open_with_retry_and_fallback(
    sinks: &Sinks,
    device_name: Option<&str>,
) -> Result<OpenedStream, String> {
    const RETRIES: u32 = 3;
    let mut last_err = String::new();

    let present = device_name.is_none_or(|name| list_device_names().iter().any(|d| d == name));
    let attempts = if present { RETRIES } else { 0 };
    if !present {
        last_err = format!(
            "input device '{}' is not connected",
            device_name.unwrap_or_default()
        );
    }

    for attempt in 0..attempts {
        if attempt > 0 {
            let delay = std::time::Duration::from_millis(300 * attempt as u64);
            crate::applog!(
                "audio: retrying '{}' in {delay:?} (attempt {})",
                device_name.unwrap_or("<default>"),
                attempt + 1
            );
            std::thread::sleep(delay);
        }
        match build_input_stream(sinks, device_name) {
            Ok(opened) => return Ok(opened),
            Err(err) => {
                crate::applog!(
                    "audio: attempt {} to open '{}' failed: {err}",
                    attempt + 1,
                    device_name.unwrap_or("<default>")
                );
                last_err = err;
            }
        }
    }

    if let Some(name) = device_name {
        crate::applog!("audio: '{name}' unavailable ({last_err}), falling back to system default");
        return build_input_stream(sinks, None).map(|opened| OpenedStream {
            fallback: true,
            ..opened
        });
    }

    Err(last_err)
}

/// Root-mean-square level of a chunk, scaled into a visually useful
/// 0.0-1.0 range for the widget's live level meter. A straight linear gain
/// (tried at 4x, then 8x) couldn't satisfy both ends at once: high enough
/// to make normal speech clearly move the meter meant loud speech
/// overshot 1.0 almost immediately, low enough to keep loud speech from
/// clipping meant normal speech barely registered above the floor. A
/// sqrt-compressed (perceptual) curve fixes that — it boosts quiet/
/// moderate input much more than loud input, so normal speech already
/// shows clear, obvious movement, while genuinely loud speech still
/// saturates to a full "every bar lit" 1.0 rather than needing to be
/// shouted. Still a rough tuning (15x gain before the compression), not a
/// calibrated measurement.
fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    (rms * 15.0).clamp(0.0, 1.0).sqrt()
}

fn build_input_stream(sinks: &Sinks, device_name: Option<&str>) -> Result<OpenedStream, String> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("input device '{name}' not found"))?,
        None => host
            .default_input_device()
            .ok_or_else(|| "no input device available".to_string())?,
    };
    let resolved_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
    let config = device.default_input_config().map_err(|e| e.to_string())?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    let sample_format = config.sample_format();
    let stream_config: StreamConfig = config.into();

    let invalid = Arc::new(AtomicBool::new(false));
    let invalid_cb = invalid.clone();
    let err_fn = move |err| {
        crate::applog!("audio stream error: {err}");
        invalid_cb.store(true, Ordering::SeqCst);
    };

    // Logs once per stream, on whichever thread's callback happens to
    // deliver the first buffer — used to measure the gap between
    // stream.play() and real audio actually flowing (Continuity/Bluetooth
    // devices can have multi-second connection latency here).
    let first_callback = Arc::new(AtomicBool::new(false));
    let note_first = move |frames: usize| {
        if !first_callback.swap(true, Ordering::SeqCst) {
            crate::applog!("audio: first callback received, {frames} frames");
        }
    };

    let sinks = sinks.clone();
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| {
                note_first(data.len());
                sinks.deliver(data, channels, sample_rate);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                note_first(data.len());
                let converted: Vec<f32> =
                    data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                sinks.deliver(&converted, channels, sample_rate);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                note_first(data.len());
                let converted: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect();
                sinks.deliver(&converted, channels, sample_rate);
            },
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| e.to_string())?;

    Ok(OpenedStream {
        stream,
        sample_rate,
        channels,
        resolved_name,
        fallback: false,
        invalid,
    })
}

fn write_wav(samples: &[f32], sample_rate: u32, channels: u16) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let path = std::env::temp_dir().join(format!("mouthpiece-{timestamp}.wav"));

    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;
    for &sample in samples {
        let clamped = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(clamped).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_zero() {
        assert_eq!(rms_level(&[0.0; 100]), 0.0);
    }

    #[test]
    fn empty_slice_is_zero() {
        assert_eq!(rms_level(&[]), 0.0);
    }

    #[test]
    fn a_quiet_tone_still_produces_a_clearly_visible_level() {
        // The whole point of the sqrt compression: even fairly quiet input
        // should already read as a clear, visible movement on the meter,
        // not a barely-there flicker near the floor.
        let quiet = vec![0.02; 100];
        let level = rms_level(&quiet);
        assert!(
            level > 0.4 && level < 1.0,
            "expected a clearly visible but unsaturated level, got {level}"
        );
    }

    #[test]
    fn a_loud_tone_clamps_to_one() {
        let loud = vec![0.9; 100];
        assert_eq!(rms_level(&loud), 1.0);
    }

    #[test]
    fn louder_input_produces_a_higher_level() {
        let quiet = vec![0.02; 100];
        let louder = vec![0.1; 100];
        assert!(rms_level(&louder) > rms_level(&quiet));
    }
    fn facts(requested: Option<&str>, resolved: &str, fallback: bool) -> StreamFacts {
        StreamFacts {
            requested: requested.map(str::to_string),
            resolved: resolved.to_string(),
            fallback,
            invalid: false,
        }
    }

    fn never_called_present(_: &str) -> bool {
        panic!("device enumeration should not run for this case")
    }

    fn never_called_default() -> Option<String> {
        panic!("default-device lookup should not run for this case")
    }

    #[test]
    fn no_stream_needs_a_build() {
        assert!(rebuild_reason(None, None, never_called_present, never_called_default).is_some());
    }

    #[test]
    fn healthy_preferred_stream_is_reused_without_probing() {
        let f = facts(Some("AIRHUG 28"), "AIRHUG 28", false);
        assert_eq!(
            rebuild_reason(
                Some(&f),
                Some("AIRHUG 28"),
                never_called_present,
                never_called_default
            ),
            None
        );
    }

    #[test]
    fn an_errored_stream_is_rebuilt() {
        let mut f = facts(Some("AIRHUG 28"), "AIRHUG 28", false);
        f.invalid = true;
        assert!(rebuild_reason(
            Some(&f),
            Some("AIRHUG 28"),
            never_called_present,
            never_called_default
        )
        .is_some());
    }

    #[test]
    fn picking_a_different_device_rebuilds() {
        let f = facts(None, "MacBook Pro Microphone", false);
        assert!(rebuild_reason(
            Some(&f),
            Some("AIRHUG 28"),
            never_called_present,
            never_called_default
        )
        .is_some());
    }

    #[test]
    fn fallback_recovers_once_the_preferred_device_returns() {
        let f = facts(Some("AIRHUG 28"), "MacBook Pro Microphone", true);
        assert!(
            rebuild_reason(Some(&f), Some("AIRHUG 28"), |_| true, never_called_default).is_some()
        );
        assert_eq!(
            rebuild_reason(Some(&f), Some("AIRHUG 28"), |_| false, never_called_default),
            None
        );
    }

    #[test]
    fn system_default_change_rebuilds_only_when_no_device_is_picked() {
        let f = facts(None, "MacBook Pro Microphone", false);
        assert!(
            rebuild_reason(Some(&f), None, never_called_present, || Some(
                "AirPods".to_string()
            ))
            .is_some()
        );
        assert_eq!(
            rebuild_reason(Some(&f), None, never_called_present, || Some(
                "MacBook Pro Microphone".to_string()
            )),
            None
        );
    }

    #[test]
    fn status_distinguishes_fallback_from_preferred_and_default() {
        assert_eq!(
            device_status(
                Some(&facts(Some("AIRHUG 28"), "MacBook Pro Microphone", true)),
                Some("AIRHUG 28")
            ),
            DeviceStatus::Fallback {
                wanted: "AIRHUG 28".into(),
                using: "MacBook Pro Microphone".into()
            }
        );
        assert_eq!(
            device_status(
                Some(&facts(Some("AIRHUG 28"), "AIRHUG 28", false)),
                Some("AIRHUG 28")
            ),
            DeviceStatus::Preferred {
                using: "AIRHUG 28".into()
            }
        );
        assert_eq!(
            device_status(
                Some(&facts(Some("AIRHUG 28"), "AIRHUG 28", true)),
                Some("AIRHUG 28")
            ),
            DeviceStatus::Preferred {
                using: "AIRHUG 28".into()
            }
        );
        assert_eq!(
            device_status(Some(&facts(None, "MacBook Pro Microphone", false)), None),
            DeviceStatus::SystemDefault {
                using: "MacBook Pro Microphone".into()
            }
        );
        assert_eq!(
            device_status(None, Some("AIRHUG 28")),
            DeviceStatus::Unavailable {
                wanted: Some("AIRHUG 28".into())
            }
        );
    }
    #[test]
    fn silence_watch_warns_after_two_quiet_seconds_and_clears_on_sound() {
        let t0 = std::time::Instant::now();
        let mut w = SilenceWatch::default();
        assert_eq!(w.observe(0.0, t0), None);
        assert_eq!(
            w.observe(0.0, t0 + std::time::Duration::from_millis(1900)),
            None
        );
        assert_eq!(
            w.observe(0.01, t0 + std::time::Duration::from_secs(2)),
            Some(true)
        );
        assert_eq!(
            w.observe(0.0, t0 + std::time::Duration::from_secs(3)),
            None,
            "warns only once"
        );
        assert_eq!(
            w.observe(0.4, t0 + std::time::Duration::from_secs(4)),
            Some(false)
        );
        assert_eq!(w.observe(0.4, t0 + std::time::Duration::from_secs(5)), None);
    }

    #[test]
    fn silence_watch_ignores_a_normal_quiet_room() {
        let t0 = std::time::Instant::now();
        let mut w = SilenceWatch::default();
        // A real mic's self-noise in a quiet room (~-65 dBFS) reads ~0.1.
        let room = rms_level(&[0.0006; 512]);
        for i in 0..100 {
            assert_eq!(
                w.observe(room, t0 + std::time::Duration::from_millis(i * 50)),
                None
            );
        }
    }

    #[test]
    fn dead_air_is_digital_silence_not_a_quiet_room() {
        assert!(is_dead_air(&[0.0; 16000]));
        assert!(is_dead_air(&[]));
        assert!(
            !is_dead_air(&[0.0006; 16000]),
            "quiet room noise is not dead air"
        );
        assert!(!is_dead_air(&[0.1; 16000]));
    }
}
