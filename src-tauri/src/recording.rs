use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use crate::app_detect::AppInfo;
use crate::audio::AudioHandle;
use crate::capture::{CaptureController, Request, Transition, Trigger};
use crate::modes;
use crate::paste::paste_text;
use crate::pipeline;
use crate::stt::WhisperEngine;

pub struct RecordingState {
    pub audio: AudioHandle,
    pub whisper: WhisperEngine,
    /// Owns "what is the mic being used for" — dictation, enrollment, or
    /// nothing. See `capture.rs`.
    pub capture: CaptureController,
    /// Held across a state transition *and* its audio start/stop command,
    /// so two threads requesting at once can't reach the audio thread in
    /// the opposite order of their transitions (a Stop landing before the
    /// Start it was meant to end would leave the stream running).
    pub capture_op: Mutex<()>,
    /// Whatever app was frontmost when the current/last recording started —
    /// used to pick a formatting mode, and doubles as "last app you were
    /// in" for the Settings quick-add UI (capturing frontmost app right
    /// before Settings opens doesn't work: opening Settings requires
    /// clicking a button in the widget, so the widget is always already
    /// frontmost by that point).
    pub active_app: Mutex<Option<AppInfo>>,
    /// One-shot override for the *next* dictation's mode, set from the
    /// widget's quick-actions flyout (see WidgetView.tsx) as a mode name —
    /// resolved via `modes::settings_for_name` at consume time, so the
    /// override carries the whole mode's settings (behavior, stt_model,
    /// llm_refinement), not just a raw behavior. Takes priority over
    /// whatever `modes::resolve_settings` would otherwise pick for the
    /// frontmost app, and is consumed (cleared) the moment a dictation uses
    /// it, so it never silently applies to a second one.
    pub mode_override: Mutex<Option<String>>,
    /// "Paste as-is" for the current dictation (widget click or the skip
    /// shortcut): skip LLM refinement, or stop it if it's running. Reset
    /// when a new dictation starts.
    pub skip_refine: std::sync::atomic::AtomicBool,
    /// What the last real paste looked like, for "scratch that" (see
    /// `undo.rs`). Length and app only, never the text.
    pub last_paste: Mutex<Option<crate::undo::LastPaste>>,
    /// Background chunk transcription for the recording in progress (see
    /// `live.rs`); taken by the stop that ends it.
    pub live: Mutex<Option<crate::live::LiveSession>>,
}

pub fn toggle_recording(app: &AppHandle, trigger: Trigger) {
    request_dictation(app, Request::ToggleDictation(trigger));
}

/// The one entry point for starting/stopping dictation from any trigger.
pub fn request_dictation(app: &AppHandle, request: Request) {
    let state = app.state::<RecordingState>();
    let _op = state.capture_op.lock().unwrap();
    match state.capture.request(request) {
        Transition::StartedDictation { session } => start_dictation(app, &state, session),
        Transition::StoppedDictation { .. } => stop_dictation(app, &state),
        Transition::DiscardedDictation { .. } => discard_dictation(app, &state),
        Transition::StartedEnrollment { .. }
        | Transition::StoppedEnrollment { .. }
        | Transition::StartedMeeting { .. }
        | Transition::StoppedMeeting { .. } => {}
        Transition::Rejected(_) => {
            // The hotkey seemingly doing nothing would look broken.
            if matches!(
                state.capture.state(),
                crate::capture::CaptureState::Meeting { .. }
            ) {
                let _ = app.emit(
                    "meeting-blocked",
                    "Dictation is paused while a meeting records".to_string(),
                );
            }
        }
    }
}

fn stop_dictation(app: &AppHandle, state: &RecordingState) {
    let stopped_at = std::time::Instant::now();
    crate::refresh_tray_indicator(app);
    let live = state.live.lock().unwrap().take();
    // Taken even if stopping fails, so it can't leak into the next recording.
    let reply = app
        .state::<crate::agents::AgentState>()
        .take_pending_reply();
    match state.audio.stop() {
        Ok(path) => {
            let _ = app.emit("recording-stopped", path.to_string_lossy().to_string());
            let app = app.clone();
            std::thread::spawn(move || {
                crate::discard::set_processing(&app, true);
                transcribe_and_paste(&app, &path, stopped_at, live, reply);
                crate::discard::set_processing(&app, false);
                // Raw voice recordings shouldn't accumulate in /tmp; history
                // keeps the transcript, not the audio.
                if let Err(err) = std::fs::remove_file(&path) {
                    crate::applog!("recording: could not delete {}: {err}", path.display());
                }
            });
        }
        Err(err) => {
            crate::discard::release_escape(app);
            let _ = app.emit("recording-error", err);
        }
    }
}

/// Esc while recording: stop without transcribing, and hold the audio so
/// Undo can still use it (see `discard.rs`).
fn discard_dictation(app: &AppHandle, state: &RecordingState) {
    crate::refresh_tray_indicator(app);
    // Dropping the live session stops its background transcription.
    drop(state.live.lock().unwrap().take());
    let reply = app
        .state::<crate::agents::AgentState>()
        .take_pending_reply();
    crate::discard::release_escape(app);
    match state.audio.stop() {
        Ok(path) => {
            crate::applog!("recording: discarded");
            let _ = app.emit("dictation-discarded", "recording");
            crate::discard::hold(app, crate::discard::Held::Recording { path, reply });
        }
        Err(err) => {
            let _ = app.emit("recording-error", err);
        }
    }
}

/// Undo of a discarded recording: transcribe and paste it as if it had
/// been stopped normally (the target app and mode were captured when it
/// started, and nothing has replaced them since).
pub fn transcribe_held_recording(
    app: &AppHandle,
    path: &std::path::Path,
    reply: Option<crate::agents::WaitingAgent>,
) {
    let _ = app.emit("recording-stopped", path.to_string_lossy().to_string());
    crate::discard::set_processing(app, true);
    crate::discard::grab_escape(app);
    transcribe_and_paste(app, path, std::time::Instant::now(), None, reply);
    crate::discard::set_processing(app, false);
}

fn current_mic_name(state: &RecordingState) -> String {
    use crate::audio::DeviceStatus::*;
    match state.audio.device_status() {
        Some(Preferred { using } | Fallback { using, .. } | SystemDefault { using }) => using,
        Some(Unavailable { .. }) | None => "your microphone".to_string(),
    }
}

fn start_dictation(app: &AppHandle, state: &RecordingState, session: u64) {
    state
        .skip_refine
        .store(false, std::sync::atomic::Ordering::Relaxed);
    // A new recording ends the chance to undo the last discard.
    crate::discard::clear(app);
    crate::discard::grab_escape(app);
    crate::refresh_tray_indicator(app);
    state.audio.start();

    let cfg = crate::config::load(app);
    *state.live.lock().unwrap() = cfg
        .live_transcribe_enabled
        .then(|| crate::live::LiveSession::start(app));
    for model in crate::llm::models_in_use(&cfg) {
        crate::llm::prewarm(&model);
    }

    // Live level meter: polls the audio thread's current RMS level (see
    // `audio::AudioHandle::current_level`) at 20fps rather than per audio
    // callback, which fires far more often than any redraw needs. Bound to
    // this session id, so it exits when *this* recording ends even if a
    // new one has already started.
    let level_app = app.clone();
    std::thread::spawn(move || {
        let state = level_app.state::<RecordingState>();
        let mut silence = crate::audio::SilenceWatch::default();
        while state.capture.is_session_active(session) {
            let level = state.audio.current_level();
            let _ = level_app.emit("audio-level", level);
            match silence.observe(level, std::time::Instant::now()) {
                Some(true) => {
                    let mic = current_mic_name(&state);
                    let access = crate::mic_permission::status();
                    crate::applog!(
                        "audio: no signal from {mic} for 2s while recording (mic access {access:?})"
                    );
                    let message = crate::mic_permission::silence_explanation(access)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!("No sound from {mic}. Is it muted? Hover to switch mics.")
                        });
                    let _ = level_app.emit("mic-silent", message);
                }
                Some(false) => {
                    crate::applog!("audio: signal resumed");
                    let _ = level_app.emit("mic-silent-cleared", ());
                }
                None => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    // Capture which app the user was in *before* showing our own widget —
    // otherwise NSWorkspace reports Mouthpiece itself as frontmost. Both
    // steps run in one main-thread closure (NSWorkspace requires the main
    // thread) so the ordering is guaranteed.
    let main_thread_app = app.clone();
    let _ = app.run_on_main_thread(move || {
        let info = crate::app_detect::frontmost_app_info();
        crate::applog!(
            "modes: frontmost app at recording start = {:?}",
            info.as_ref().map(|i| (&i.bundle_id, &i.name))
        );
        let state = main_thread_app.state::<RecordingState>();
        // Names from the project open in a frontmost terminal or editor are
        // looked up in the background so they're ready for the transcript.
        crate::project::begin(
            &main_thread_app,
            info.as_ref().map(|i| i.bundle_id.clone()),
            crate::app_detect::frontmost_pid(),
        );
        *state.active_app.lock().unwrap() = info;

        // `show()` only, not `set_focus()` — stealing keyboard focus made
        // the paste land in the widget instead of the app being dictated
        // into.
        if let Some(window) = main_thread_app.get_webview_window("widget") {
            let _ = window.show();
        }
    });

    let _ = app.emit("recording-started", ());
}

/// The mode a dictation resolves to, and the speech model it implies.
pub(crate) struct ResolvedMode {
    pub cfg: crate::config::AppConfig,
    pub settings: modes::ResolvedSettings,
    pub app_name: Option<String>,
    /// The mode's `stt_model` override (id + path), if it has one that's
    /// installed.
    pub model: Option<(String, std::path::PathBuf)>,
}

/// Mode (and its stt_model override, if any) has to be resolved *before*
/// transcribing — which model to transcribe with depends on it. Casing-
/// command detection still happens after, on the resulting text, since
/// that's orthogonal to which model produced it. `consume_override` takes
/// the one-shot next-dictation override; chunks transcribed mid-recording
/// only peek at it, so the dictation's final resolution still sees it.
/// A reply to a waiting coding agent is formatted for talking to the
/// agent, not for the app that was in front when recording started, and
/// is credited to the agent's app.
pub(crate) fn resolve_mode(
    app: &AppHandle,
    consume_override: bool,
    reply: Option<&crate::agents::WaitingAgent>,
) -> ResolvedMode {
    let state = app.state::<RecordingState>();
    let active_app = state.active_app.lock().unwrap().clone();
    let (bundle_id, app_name) = match &reply {
        Some(agent) => (None, agent.app_name.clone()),
        None => (
            active_app.as_ref().map(|info| info.bundle_id.clone()),
            active_app.map(|info| info.name),
        ),
    };

    let mut cfg = crate::config::load(app);
    if reply.is_some() {
        // "Send it" / "press enter" ends a reply with Enter without the user
        // having turned the global voice command on.
        cfg.press_enter_enabled = true;
    }
    let mut settings = modes::resolve_settings(bundle_id.as_deref(), &cfg.modes);
    let override_name = if consume_override {
        state.mode_override.lock().unwrap().take()
    } else {
        state.mode_override.lock().unwrap().clone()
    };
    if let Some(override_name) = override_name {
        match modes::settings_for_name(&override_name, &cfg.modes) {
            Some(override_settings) => {
                if consume_override {
                    crate::applog!(
                        "modes: next-dictation override {override_name:?} applied over resolved {:?}",
                        settings.mode
                    );
                }
                settings = override_settings;
            }
            None => {
                if consume_override {
                    crate::applog!(
                        "modes: next-dictation override {override_name:?} no longer exists, ignoring"
                    );
                }
            }
        }
    }
    let model = settings
        .stt_model
        .as_ref()
        .and_then(|id| crate::models::resolve_model_path(app, id).map(|path| (id.clone(), path)));
    if consume_override {
        crate::applog!(
            "modes: bundle_id={bundle_id:?} modes={} resolved_mode={:?} stt_model_override={:?} (resolved={})",
            cfg.modes.len(),
            settings.mode,
            settings.stt_model,
            model.is_some(),
        );
    }
    ResolvedMode {
        cfg,
        settings,
        app_name,
        model,
    }
}

fn transcribe_and_paste(
    app: &AppHandle,
    wav_path: &std::path::Path,
    stopped_at: std::time::Instant,
    live: Option<crate::live::LiveSession>,
    reply: Option<crate::agents::WaitingAgent>,
) {
    let state = app.state::<RecordingState>();

    let ResolvedMode {
        cfg,
        settings,
        app_name,
        model: model_override,
        ..
    } = resolve_mode(app, true, reply.as_ref());

    // Read once, up front, from the wav header (not the decoded/resampled
    // samples above) — feeds the words-per-minute stat in Insights.
    let duration_ms = crate::stt::wav_duration_ms(wav_path);

    let stt_started = std::time::Instant::now();
    let samples = crate::stt::load_samples_16k_mono(wav_path);
    let transcribed = match samples {
        Ok(samples) if crate::audio::is_dead_air(&samples) => {
            let mic = current_mic_name(&state);
            let access = crate::mic_permission::status();
            crate::applog!(
                "audio: recording from {mic} was silent (no signal at all, mic access {access:?}), \
                 skipping transcription"
            );
            let message = crate::mic_permission::silence_explanation(access)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "{mic} sent no audio. Is it muted, or is the input level at zero in \
                         System Settings > Sound? Hover to switch mics."
                    )
                });
            let _ = app.emit("transcript-error", message);
            return;
        }
        Ok(samples) => {
            let outcome = live.and_then(|session| session.finish(samples.clone()));
            match crate::live::plan_stop(samples.len(), outcome.as_ref()) {
                crate::live::StopPlan::Chunked => {
                    let outcome = outcome.expect("a chunked plan has an outcome");
                    transcribe_tail(app, &state, outcome, model_override.clone(), stopped_at)
                        .or_else(|reason| {
                            crate::applog!("live: {reason}, transcribing the whole recording");
                            transcribe_whole(app, &state, samples, model_override)
                        })
                }
                crate::live::StopPlan::WholeBuffer(reason) => {
                    if outcome.is_some() {
                        crate::applog!("live: transcribing the whole recording ({reason})");
                    }
                    transcribe_whole(app, &state, samples, model_override)
                }
            }
        }
        Err(err) => Err(err),
    };

    let stt_ms = Some(stt_started.elapsed().as_millis() as u64);

    match transcribed {
        Ok(text) if !text.is_empty() => {
            // Raw model output, logged before any pipeline pass touches it
            // (punctuation/backtrack/trigger-stripping/mode/LLM all mutate
            // `text` below) — the only place to see what Whisper actually
            // heard, needed to tell a real STT mis-transcription apart from
            // a bug in one of those passes when a voice command misfires.
            crate::applog!("whisper: raw transcript={}", crate::logging::said(&text));

            let text = crate::stt::strip_non_speech(&text);
            if text.is_empty() {
                crate::applog!(
                    "whisper: transcript was only non-speech annotations, nothing to paste"
                );
                let _ = app.emit("transcript-error", "no speech detected".to_string());
                return;
            }

            let text = if reply.is_some() {
                crate::agents::normalize_reply(&text)
            } else {
                text
            };
            process_transcript(
                app,
                text,
                TranscriptContext {
                    cfg: &cfg,
                    settings: &settings,
                    app_name,
                    duration_ms,
                    paste_guard: None,
                    stopped_at: Some(stopped_at),
                    stt_ms,
                    project: crate::project::current(),
                    reply_target: reply,
                },
            );
        }
        Ok(_) => {
            let _ = app.emit("transcript-error", "no speech detected".to_string());
        }
        Err(err) => {
            let _ = app.emit("transcript-error", err);
        }
    }
}

/// The original path: Isolated Voice, then Whisper over the whole recording,
/// with progress for the full length.
fn transcribe_whole(
    app: &AppHandle,
    state: &RecordingState,
    samples: Vec<f32>,
    model_override: Option<(String, std::path::PathBuf)>,
) -> Result<String, String> {
    let samples = crate::isolate::apply(app, samples);
    let progress_app = app.clone();
    let result =
        state
            .whisper
            .transcribe_samples_with_progress(&samples, model_override, &move |percent| {
                let _ = progress_app.emit("transcribe-progress", percent);
            });
    if result.is_ok() {
        let _ = app.emit("transcribe-progress", 100u8);
    }
    result
}

/// Finishes a dictation whose earlier chunks were transcribed while it was
/// being spoken: only the speech after the last cut is left, and progress
/// covers just that. `Err` carries why the caller should fall back to the
/// whole recording.
fn transcribe_tail(
    app: &AppHandle,
    state: &RecordingState,
    outcome: crate::live::LiveOutcome,
    model_override: Option<(String, std::path::PathBuf)>,
    stopped_at: std::time::Instant,
) -> Result<String, String> {
    let tail_started = std::time::Instant::now();
    let chunks = outcome.texts.len();
    let mut parts = outcome.texts.clone();
    if let Some(tail) = outcome.tail.as_ref() {
        let tail_s = tail.len() as f32 / crate::stt::WHISPER_SAMPLE_RATE as f32;
        let tail = crate::isolate::apply(app, tail.clone());
        let progress_app = app.clone();
        let text = state
            .whisper
            .transcribe_samples_in_context(
                &tail,
                model_override,
                &crate::live::join_texts(&parts),
                &move |percent| {
                    let _ = progress_app.emit("transcribe-progress", percent);
                },
            )
            .map_err(|err| format!("tail of {tail_s:.1}s failed ({err})"))?;
        parts.push(crate::stt::strip_non_speech(&text));
    }
    let joined = crate::live::join_texts(&parts);
    if joined.is_empty() {
        return Err("chunks and tail were empty".to_string());
    }
    let _ = app.emit("transcribe-progress", 100u8);
    crate::applog!(
        "live: chunks={chunks} tail={} audio={:.1}s transcribed={:.1}s skipped_silence={:.1}s drain_ms={} tail_ms={} stop_to_text_ms={}",
        outcome.tail.is_some(),
        outcome.total_samples as f32 / crate::stt::WHISPER_SAMPLE_RATE as f32,
        outcome.kept_samples as f32 / crate::stt::WHISPER_SAMPLE_RATE as f32,
        outcome.skipped_seconds(),
        outcome.drain_ms,
        tail_started.elapsed().as_millis(),
        stopped_at.elapsed().as_millis(),
    );
    Ok(joined)
}

/// Everything a transcript needs besides its text to be formatted and
/// delivered — shared by push-to-talk dictation and ambient listening.
pub(crate) struct TranscriptContext<'a> {
    pub cfg: &'a crate::config::AppConfig,
    pub settings: &'a modes::ResolvedSettings,
    pub app_name: Option<String>,
    pub duration_ms: Option<u64>,
    /// Bundle id that must still be frontmost at delivery time for the text
    /// to be pasted (ambient only); `None` pastes unconditionally.
    pub paste_guard: Option<String>,
    /// When the user stopped recording, for end-to-end latency. `None` for
    /// ambient dictation, which has no explicit stop.
    pub stopped_at: Option<std::time::Instant>,
    /// How long Whisper (plus voice isolation) took.
    pub stt_ms: Option<u64>,
    /// Names from the project being dictated into, if one was found.
    pub project: Option<std::sync::Arc<crate::project::Project>>,
    /// A waiting coding agent this dictation is a reply to: delivered by
    /// hopping to its window instead of pasting where focus is.
    pub reply_target: Option<crate::agents::WaitingAgent>,
}

fn ms_or_dash(ms: Option<u64>) -> String {
    ms.map_or_else(|| "-".to_string(), |ms| format!("{ms}ms"))
}

/// Runs a cleaned transcript through the formatting pipeline (lists,
/// punctuation, backtrack, voice commands, snippets, casing, boilerplate,
/// mode formatting, LLM refinement) and delivers it.
pub(crate) fn process_transcript(app: &AppHandle, text: String, ctx: TranscriptContext) {
    let TranscriptContext {
        cfg,
        settings,
        app_name,
        duration_ms,
        ..
    } = &ctx;
    let (cfg, settings, app_name, duration_ms) = (*cfg, *settings, app_name.clone(), *duration_ms);
    let pipeline::Formatted {
        text: formatted,
        mode_label,
        press_enter: should_press_enter,
        append_clipboard: should_append_clipboard,
        attach_error: should_attach_error,
        undo_last,
        features_used,
        spoken_words,
        llm_ms,
        unrefined,
    } = pipeline::format_in_project(
        text,
        cfg,
        settings,
        ctx.project.as_deref(),
        &pipeline::OllamaRefiner {
            on_progress: &|percent| {
                let _ = app.emit("refine-progress", percent);
            },
            cancel: &app.state::<RecordingState>().skip_refine,
        },
        &mut |model| {
            let _ = app.emit("refining-started", model.to_string());
        },
    );

    if undo_last {
        if ctx.reply_target.is_some() {
            // In a reply, "scratch that" cancels the reply. Undoing would
            // hit whatever app is in front, which isn't the agent's.
            crate::applog!("agents: reply cancelled by \"scratch that\"");
            let _ = app.emit("transcript-copied", "Reply cancelled".to_string());
        } else {
            undo_last_paste(app);
        }
        return;
    }

    // "Attach the error": the error block goes in after the dictated text,
    // fenced, and isn't run through mode formatting or the LLM either.
    // Read before the paste below replaces the clipboard.
    let mut note: Option<&'static str> = None;
    let formatted = if should_attach_error {
        match find_error_block(app) {
            Some(block) => crate::error_context::attach(&formatted, &block),
            None => {
                note = Some("No error found to attach");
                formatted
            }
        }
    } else {
        formatted
    };

    // Applied uniformly after whichever branch above resolved
    // `formatted` — clipboard content is appended raw, not run
    // through mode formatting or LLM refinement, the same way a
    // snippet body would be. A trigger with nothing on the
    // clipboard (or a non-text clipboard) leaves `formatted`
    // unchanged rather than pasting a Rust error string.
    let (formatted, unrefined) = if should_append_clipboard {
        match crate::clipboard::read_clipboard_text() {
            Some(clip) => (
                crate::clipboard::append(&formatted, &clip),
                unrefined.map(|u| crate::clipboard::append(&u, &clip)),
            ),
            None => {
                crate::applog!("clipboard: append triggered but clipboard was empty or unreadable");
                (formatted, unrefined)
            }
        }
    } else {
        (formatted, unrefined)
    };

    // A fully-consumed "press enter"-only utterance formats down
    // to an empty string — nothing to paste/copy in that case
    // (would otherwise silently overwrite the clipboard with
    // nothing), but Enter still needs to fire below.
    // Ambient dictation passes the app that was frontmost when the speech
    // arrived; if focus moved during transcription, pasting would type into
    // whatever took focus, so the text goes to the clipboard instead.
    let focus_moved = ctx.paste_guard.as_deref().is_some_and(|expected| {
        let now = crate::app_detect::frontmost_bundle_id_blocking(app);
        if now.as_deref() != Some(expected) {
            crate::applog!("paste guard: frontmost changed from {expected:?} to {now:?}, copying instead of pasting");
            true
        } else {
            false
        }
    });
    // Esc while this was being prepared: keep the text for Undo instead.
    if app
        .state::<crate::discard::DiscardState>()
        .cancel_requested()
    {
        crate::discard::hold(app, crate::discard::Held::Text(formatted));
        return;
    }

    // A reply goes to the waiting agent's window (or the clipboard if that
    // can't be reached safely); Enter, if asked for, is pressed over there.
    let reply = ctx.reply_target.as_ref().map(|target| {
        crate::agents::deliver_reply(app, target, &formatted, should_press_enter, cfg.copy_only)
    });
    let copy_only = cfg.copy_only || focus_moved || reply.as_ref().is_some_and(|r| r.copied);
    let deliver_started = std::time::Instant::now();
    let deliver = if let Some(reply) = &reply {
        reply.result.clone()
    } else if formatted.is_empty() {
        Ok(())
    } else if copy_only {
        crate::paste::copy_text(&formatted)
    } else {
        paste_text(&formatted)
    };
    let latency_ms = ctx.stopped_at.map(|t| t.elapsed().as_millis() as u64);
    crate::applog!(
        "perf: stt={} llm={} deliver={}ms total={}",
        ms_or_dash(ctx.stt_ms),
        ms_or_dash(llm_ms),
        deliver_started.elapsed().as_millis(),
        ms_or_dash(latency_ms),
    );

    match deliver {
        Ok(()) => {
            // Copy-only means the user opted out of synthetic
            // keystrokes entirely (that's the point of the mode,
            // and why it doesn't need Accessibility permission),
            // so "press enter" is skipped rather than sending one
            // anyway.
            // A reply's Enter was already pressed in the agent's window.
            let submitted = should_press_enter && !copy_only && reply.is_none();
            if submitted {
                if let Err(err) = crate::paste::press_enter() {
                    crate::applog!("press_enter: failed to simulate Enter: {err}");
                }
            }

            // Remember the paste for "scratch that". Copies don't count,
            // and an Enter after it means the line is gone from the input.
            // A reply landed in another window and focus has moved back,
            // so there is nothing here to undo.
            if !copy_only && reply.is_none() {
                let state = app.state::<RecordingState>();
                if !formatted.is_empty() {
                    let bundle = crate::app_detect::frontmost_bundle_id_blocking(app);
                    *state.last_paste.lock().unwrap() = Some(crate::undo::LastPaste::new(
                        &formatted,
                        bundle,
                        submitted,
                        std::time::Instant::now(),
                    ));
                } else if submitted {
                    if let Some(last) = state.last_paste.lock().unwrap().as_mut() {
                        last.submitted = true;
                    }
                }
            }

            // Cloned before the moves into `append_entry` below —
            // the webhook payload needs its own copies of whatever
            // it fires after.
            let webhook_app_name = app_name.clone();
            let webhook_mode = mode_label.clone();

            // Only log to history what actually reached the user —
            // a failed paste/copy shouldn't silently show up as
            // history, and neither should an empty "press enter
            // only" utterance with nothing to show.
            let timestamp_ms = if !formatted.is_empty() {
                Some(crate::history::append_entry(
                    app,
                    crate::history::NewEntry {
                        text: formatted.clone(),
                        app_name,
                        mode: Some(mode_label),
                        duration_ms,
                        features_used,
                        spoken_words,
                        latency_ms,
                        original_text: unrefined,
                    },
                ))
            } else {
                None
            };

            // Mirrors the history-logging condition above — an
            // empty "press enter only" utterance has no real
            // transcript to send downstream either.
            if let (Some(webhook_url), Some(timestamp_ms)) = (cfg.webhook_url.clone(), timestamp_ms)
            {
                crate::webhook::send_entry(
                    webhook_url,
                    crate::webhook::WebhookPayload {
                        timestamp_ms,
                        text: formatted.clone(),
                        // Known v1 gap: the journal summary (if
                        // enabled) is generated asynchronously
                        // below and isn't available yet — see
                        // BACKLOG.md.
                        summary: None,
                        app_name: webhook_app_name,
                        mode: Some(webhook_mode),
                    },
                );
            }

            // Journal summarization happens after the entry is
            // already saved and the transcript already delivered —
            // a slow/unreachable Ollama call here should never add
            // latency to the thing the user is actually waiting on.
            if let (true, Some(timestamp_ms)) = (cfg.journal_enabled, timestamp_ms) {
                let journal_app = app.clone();
                let journal_text = formatted.clone();
                let journal_model = cfg.llm_model.clone();
                std::thread::spawn(move || {
                    match crate::llm::summarize_for_journal(&journal_text, &journal_model) {
                        Some(Ok(summary)) => {
                            crate::history::set_entry_summary(&journal_app, timestamp_ms, summary);
                        }
                        Some(Err(err)) => {
                            crate::applog!("journal: summarization failed: {err}");
                        }
                        None => {}
                    }
                });
            }

            if let Some(reply) = &reply {
                let _ = app.emit("transcript-copied", reply.notice.clone());
            } else if focus_moved {
                let _ = app.emit(
                    "transcript-copied",
                    "Copied — you switched apps before it could paste".to_string(),
                );
            } else if copy_only && !formatted.is_empty() {
                let _ = app.emit("transcript-copied", "Copied to clipboard".to_string());
            } else if let Some(note) = note {
                // `transcript-copied` is just the widget's quiet,
                // non-error status line.
                let _ = app.emit("transcript-copied", note.to_string());
            } else {
                let _ = app.emit("transcript-ready", formatted);
            }
        }
        Err(err) => {
            let _ = app.emit("transcript-error", err);
        }
    }
}

/// The most recent error output, for "attach the error": the frontmost
/// terminal's visible text if it's a terminal that exposes it, else the
/// clipboard if that looks like an error. Logs line counts only.
fn find_error_block(app: &AppHandle) -> Option<String> {
    if let Some((bundle_id, pid)) = crate::app_detect::frontmost_bundle_and_pid_blocking(app) {
        if crate::error_context::is_terminal_app(&bundle_id) {
            // Accessibility calls can hang on a busy app; give up after 3s.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::error_context::read_terminal_text(pid));
            });
            match rx.recv_timeout(std::time::Duration::from_secs(3)) {
                Ok(Some(text)) => {
                    let found = crate::error_context::extract_error_block(&text);
                    crate::applog!(
                        "attach error: read {} terminal lines, error block: {}",
                        text.lines().count(),
                        found.as_ref().map_or("none".to_string(), |b| format!(
                            "{} lines",
                            b.lines().count()
                        ))
                    );
                    if found.is_some() {
                        return found;
                    }
                }
                Ok(None) => crate::applog!(
                    "attach error: terminal text not readable via Accessibility, trying clipboard"
                ),
                Err(_) => crate::applog!("attach error: terminal read timed out, trying clipboard"),
            }
        }
    }
    let clip = crate::clipboard::read_clipboard_text()?;
    let found = crate::error_context::clipboard_error_text(&clip);
    crate::applog!(
        "attach error: clipboard has {} lines, error-like: {}",
        clip.lines().count(),
        found.is_some()
    );
    found
}

/// "Scratch that" as a whole dictation: undo the last paste if that can be
/// done safely (see `undo.rs`), and tell the widget either way.
fn undo_last_paste(app: &AppHandle) {
    let state = app.state::<RecordingState>();
    let last = state.last_paste.lock().unwrap().clone();
    let frontmost = crate::app_detect::frontmost_bundle_id_blocking(app);
    let plan = crate::undo::plan(
        last.as_ref(),
        frontmost.as_deref(),
        std::time::Instant::now(),
    );
    crate::applog!("undo: plan={plan:?}");
    const UNDID: &str = "Undid the last paste";
    let result = match plan {
        crate::undo::Plan::CmdZ => crate::paste::send_undo().map(|()| UNDID),
        crate::undo::Plan::Backspaces(n) => crate::paste::send_backspaces(n).map(|()| UNDID),
        crate::undo::Plan::Refuse(note) => Ok(note),
    };
    match result {
        Ok(note) => {
            if note == UNDID {
                // One undo per paste: a second "scratch that" must not
                // reach further back.
                *state.last_paste.lock().unwrap() = None;
            }
            let _ = app.emit("transcript-copied", note.to_string());
        }
        Err(err) => {
            let _ = app.emit("transcript-error", err);
        }
    }
}

/// "Paste as-is": while recording, stops and skips refinement; while
/// refining, stops the LLM and pastes the unrefined text now. From the
/// widget's refining bar or the skip shortcut (hotkey + Shift).
pub fn paste_as_is(app: &AppHandle) {
    let state = app.state::<RecordingState>();
    state
        .skip_refine
        .store(true, std::sync::atomic::Ordering::Relaxed);
    crate::applog!("llm: paste as-is requested");
    if matches!(
        state.capture.state(),
        crate::capture::CaptureState::Dictating { .. }
    ) {
        request_dictation(app, Request::StopDictation(Trigger::Hotkey));
    }
}

#[tauri::command]
pub fn paste_as_is_command(app: AppHandle) {
    paste_as_is(&app);
}

#[tauri::command]
pub fn toggle_recording_command(app: AppHandle) {
    toggle_recording(&app, Trigger::Button);
}

/// Which mic capture is actually using (or falling back from). Read once by
/// the widget on mount; changes after that arrive as `audio-device-status`.
#[tauri::command]
pub fn get_audio_device_status(
    state: tauri::State<RecordingState>,
) -> Option<crate::audio::DeviceStatus> {
    state.audio.device_status()
}

// Off the main thread: sync commands run on it, and this one does I/O.
#[tauri::command(async)]
pub fn list_input_devices() -> Vec<String> {
    crate::audio::list_device_names()
}

/// Whichever device recording will actually use next: the explicit
/// selection if one was made, otherwise whatever cpal resolves as default.
#[tauri::command]
pub fn get_active_input_device(state: tauri::State<RecordingState>) -> Option<String> {
    state
        .audio
        .selected_device()
        .or_else(crate::audio::default_device_name)
}

#[tauri::command]
pub fn set_input_device(app: AppHandle, name: Option<String>, state: tauri::State<RecordingState>) {
    state.audio.set_device(name.clone());

    let mut cfg = crate::config::load(&app);
    cfg.input_device = name;
    let _ = crate::config::save(&app, &cfg);
}

#[tauri::command]
pub fn get_vocabulary(app: AppHandle) -> Vec<String> {
    crate::config::load(&app).vocabulary
}

#[tauri::command]
pub fn set_vocabulary(app: AppHandle, terms: Vec<String>, state: tauri::State<RecordingState>) {
    state.whisper.set_vocabulary(terms.clone());

    let mut cfg = crate::config::load(&app);
    cfg.vocabulary = terms;
    let _ = crate::config::save(&app, &cfg);
}

#[tauri::command]
pub fn get_copy_only(app: AppHandle) -> bool {
    crate::config::load(&app).copy_only
}

#[tauri::command]
pub fn set_copy_only(app: AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.copy_only = enabled;
    let _ = crate::config::save(&app, &cfg);
}

#[tauri::command]
pub fn get_isolated_voice_enabled(app: AppHandle) -> bool {
    crate::config::load(&app).isolated_voice_enabled
}

#[tauri::command]
pub fn set_isolated_voice_enabled(app: AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.isolated_voice_enabled = enabled;
    let _ = crate::config::save(&app, &cfg);
}

/// Set (or clear, with `None`) the one-shot mode override for the next
/// dictation — see `RecordingState::mode_override`. Not persisted to
/// config: this is deliberately session/one-shot state, not a setting.
#[tauri::command]
pub fn set_next_mode_override(mode_name: Option<String>, state: tauri::State<RecordingState>) {
    *state.mode_override.lock().unwrap() = mode_name;
}

#[tauri::command]
pub fn get_next_mode_override(state: tauri::State<RecordingState>) -> Option<String> {
    state.mode_override.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_press_enter_enabled(app: AppHandle) -> bool {
    crate::config::load(&app).press_enter_enabled
}

#[tauri::command]
pub fn set_press_enter_enabled(app: AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.press_enter_enabled = enabled;
    let _ = crate::config::save(&app, &cfg);
}

#[derive(serde::Serialize)]
pub struct FrontmostAppPayload {
    pub bundle_id: String,
    pub name: String,
    pub icon_data_uri: Option<String>,
}

/// The app the most recent recording was started in — used by Settings to
/// offer "add a mode rule for the app you just came from".
#[tauri::command]
pub fn get_last_frontmost_app(state: tauri::State<RecordingState>) -> Option<FrontmostAppPayload> {
    state
        .active_app
        .lock()
        .unwrap()
        .as_ref()
        .map(|info| FrontmostAppPayload {
            bundle_id: info.bundle_id.clone(),
            name: info.name.clone(),
            icon_data_uri: info.icon_data_uri.clone(),
        })
}
