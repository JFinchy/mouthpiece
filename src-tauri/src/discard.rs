//! Pausing, deleting and cancelling a dictation.
//!
//! Esc while recording pauses it (nothing is lost) and the widget asks what
//! to do: Resume, Transcribe (the dictation shortcut), or Delete (Esc
//! again). The widget's ✕ while the text is being prepared cancels the
//! paste; the text goes to the clipboard instead, so nothing is lost there
//! either.
//!
//! Esc is a global hotkey only while recording, and every press is passed
//! on to the app in front, so Mouthpiece reacts to Esc without taking it
//! away from anything else.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};

enum KeyOp {
    Register(Shortcut),
    Unregister(Shortcut),
    /// Hand an Esc press on to the frontmost app (see `forward_escape`).
    ForwardEscape,
}

/// Registering a shortcut waits for the main thread. Callers here can hold
/// the recording lock the main thread may itself be waiting on, so every
/// change goes through one worker thread that holds nothing (and keeps the
/// changes in order).
fn key_op(app: &AppHandle, op: KeyOp) {
    static WORKER: std::sync::OnceLock<Mutex<std::sync::mpsc::Sender<KeyOp>>> =
        std::sync::OnceLock::new();
    let sender = WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<KeyOp>();
        let app = app.clone();
        std::thread::spawn(move || {
            for op in rx {
                let shortcuts = app.global_shortcut();
                match op {
                    KeyOp::Register(key) => {
                        if !shortcuts.is_registered(key) {
                            if let Err(err) = shortcuts.register(key) {
                                crate::applog!("discard: couldn't register {key:?}: {err}");
                            }
                        }
                    }
                    KeyOp::Unregister(key) => {
                        if shortcuts.is_registered(key) {
                            let _ = shortcuts.unregister(key);
                        }
                    }
                    KeyOp::ForwardEscape => {
                        // Our own hotkey would catch the copy, so step aside
                        // while it's sent. Ops run in order, so a later
                        // Unregister still wins.
                        let held = shortcuts.is_registered(escape());
                        if held {
                            let _ = shortcuts.unregister(escape());
                        }
                        forward_escape();
                        if held {
                            let _ = shortcuts.register(escape());
                        }
                    }
                }
            }
        });
        Mutex::new(tx)
    });
    let _ = sender.lock().unwrap().send(op);
}

/// Sends a synthetic Esc press to whatever app is in front. Needs the
/// Accessibility permission pasting already uses; without it macOS drops
/// the event, which leaves things as they were before forwarding existed.
fn forward_escape() {
    use rdev::{simulate, EventType, Key};
    // A beat for the hotkey to be gone before the copy arrives.
    std::thread::sleep(std::time::Duration::from_millis(15));
    let _ = simulate(&EventType::KeyPress(Key::Escape));
    std::thread::sleep(std::time::Duration::from_millis(15));
    let _ = simulate(&EventType::KeyRelease(Key::Escape));
    std::thread::sleep(std::time::Duration::from_millis(15));
}

pub fn escape() -> Shortcut {
    Shortcut::new(None, Code::Escape)
}

#[derive(Default)]
pub struct DiscardState {
    /// A paste is being prepared (transcribing or refining).
    processing: AtomicBool,
    /// The paste was cancelled while processing: copy instead of pasting.
    cancel_requested: AtomicBool,
}

impl DiscardState {
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }
}

/// Registers Esc while a dictation is being recorded. Skipped if the
/// user's own hotkey is Esc.
pub fn grab_escape(app: &AppHandle) {
    if escape_is_users_hotkey(app) {
        return;
    }
    key_op(app, KeyOp::Register(escape()));
}

pub fn release_escape(app: &AppHandle) {
    if escape_is_users_hotkey(app) {
        return;
    }
    key_op(app, KeyOp::Unregister(escape()));
}

fn escape_is_users_hotkey(app: &AppHandle) -> bool {
    let current = app
        .state::<crate::shortcut::PushToTalkState>()
        .current
        .lock()
        .unwrap()
        .clone();
    [current.clone(), crate::shortcut::skip_variant(&current)]
        .iter()
        .filter_map(|c| crate::shortcut::to_shortcut(c).ok())
        .any(|s| s == escape())
}

/// Called when a recording's processing starts and ends.
pub fn set_processing(app: &AppHandle, processing: bool) {
    let state = discard_state(app);
    state.processing.store(processing, Ordering::SeqCst);
    state.cancel_requested.store(false, Ordering::SeqCst);
}

fn discard_state(app: &AppHandle) -> tauri::State<'_, DiscardState> {
    app.state::<DiscardState>()
}

/// What Esc does to a dictation, given whether one is recording and
/// whether it's paused.
#[derive(Debug, PartialEq, Eq)]
enum EscapeAction {
    Pause,
    Delete,
    Nothing,
}

fn escape_action(dictating: bool, paused: bool) -> EscapeAction {
    match (dictating, paused) {
        (true, false) => EscapeAction::Pause,
        (true, true) => EscapeAction::Delete,
        (false, _) => EscapeAction::Nothing,
    }
}

fn is_dictating(app: &AppHandle) -> bool {
    let recording = app.state::<crate::recording::RecordingState>();
    matches!(
        recording.capture.state(),
        crate::capture::CaptureState::Dictating { .. }
    )
}

/// Esc (the global hotkey): pause, or delete if already paused, then pass
/// the key on to the app in front.
pub fn on_escape(app: &AppHandle) {
    let paused = app
        .state::<crate::recording::RecordingState>()
        .audio
        .is_paused();
    match escape_action(is_dictating(app), paused) {
        EscapeAction::Pause => pause(app),
        EscapeAction::Delete => delete(app),
        EscapeAction::Nothing => {}
    }
    key_op(app, KeyOp::ForwardEscape);
}

/// Stop recording audio without ending the dictation.
pub fn pause(app: &AppHandle) {
    if !is_dictating(app) {
        return;
    }
    let recording = app.state::<crate::recording::RecordingState>();
    if recording.audio.is_paused() {
        return;
    }
    recording.audio.set_paused(true);
    crate::applog!("recording: paused");
    let _ = app.emit("dictation-paused", ());
}

pub fn resume(app: &AppHandle) {
    if !is_dictating(app) {
        return;
    }
    let recording = app.state::<crate::recording::RecordingState>();
    if !recording.audio.is_paused() {
        return;
    }
    recording.audio.set_paused(false);
    crate::applog!("recording: resumed");
    let _ = app.emit("dictation-resumed", ());
}

/// End the dictation without transcribing it.
pub fn delete(app: &AppHandle) {
    crate::recording::request_dictation(app, crate::capture::Request::DiscardDictation);
}

/// The widget's ✕ while the text is being prepared: copy it rather than
/// paste it.
pub fn cancel_paste(app: &AppHandle) {
    let state = discard_state(app);
    if !state.processing.load(Ordering::SeqCst) {
        return;
    }
    crate::applog!("discard: paste cancelled while processing");
    state.cancel_requested.store(true, Ordering::SeqCst);
    // Stop the LLM now rather than waiting for it; the text that was ready
    // goes to the clipboard.
    app.state::<crate::recording::RecordingState>()
        .skip_refine
        .store(true, Ordering::Relaxed);
    let _ = app.emit("dictation-discarded", "cancelled");
}

#[tauri::command]
pub fn pause_dictation(app: AppHandle) {
    pause(&app);
}

#[tauri::command]
pub fn resume_dictation(app: AppHandle) {
    resume(&app);
}

#[tauri::command]
pub fn delete_dictation(app: AppHandle) {
    delete(&app);
}

#[tauri::command]
pub fn cancel_paste_command(app: AppHandle) {
    cancel_paste(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_escape_pauses_and_the_second_deletes() {
        assert_eq!(escape_action(true, false), EscapeAction::Pause);
        assert_eq!(escape_action(true, true), EscapeAction::Delete);
        assert_eq!(escape_action(false, false), EscapeAction::Nothing);
        assert_eq!(escape_action(false, true), EscapeAction::Nothing);
    }
}
