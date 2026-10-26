# Always-on listening + capture/device groundwork

Branch: `main` (built directly on main, 2026-10-05)
Files changed: `ambient.rs` (new), `capture.rs` (new), `audio.rs`,
`recording.rs`, `voice_isolation.rs`, `isolate.rs`, `stt.rs`, `config.rs`,
`history.rs`, `doubletap.rs`, `app_detect.rs`, `lib.rs`, `WidgetView.tsx`,
`SettingsView.tsx`, `resources/silero_vad.onnx` (new)

## What to verify

Hands-free dictation ("Mouthpiece, ..."), plus the plumbing under it:
one state machine for every recording trigger, a visible warning when your
picked mic is missing, and fixes for pasted `[BLANK_AUDIO]`-style tags.
Unit tests cover the logic; only a real mic and real speech exercise VAD
thresholds and voice matching.

## Steps

1. `bun run tauri dev`. Watch `~/Library/Application Support/dev.mouthpiece/app.log`
   (`tail -f`) throughout.
2. **Mic fallback**: with AIRHUG 28 selected but disconnected, the widget's
   idle label reads "Using MacBook Pro Microphone" in amber; hover shows why.
   Connect the headset, dictate once: label returns to "Ready" and the log
   shows `rebuilding stream (preferred device is back)`.
3. **Triggers**: dictate via hotkey, double-tap Fn, widget button, tray
   menu, and `open mouthpiece://start-recording` then `stop-recording`.
   Each pastes normally; the log shows `capture: Idle -> Dictating...`.
   Run `open mouthpiece://start-recording` twice: the second is logged as
   rejected, not a toggle-off.
4. **Non-speech tags**: press the hotkey and stay silent (or just click the
   mouse). Widget says "no speech detected"; nothing is pasted.
5. **Ambient**: Settings → Voice Isolation → enroll your voice, then turn on
   "Always-on listening". Tray icon gets a green dot; widget reads
   "Listening for “Mouthpiece”".
6. In a text field, say "Mouthpiece, this is a hands-free test." It pastes
   "This is a hands-free test." Say a sentence *without* the wake phrase:
   nothing happens (log: `no wake phrase, ignored`).
7. Have someone else (or a video) say "Mouthpiece, hello". It shouldn't
   paste (log: `rejected (voice match ... below threshold)`).
8. Say "Mouthpiece, write this down", then immediately click into another
   app. The widget says it copied instead of pasting.
9. Say "Mouthpiece, go to sleep" (widget: "Asleep", dot clears), dictate
   something (ignored), then "Mouthpiece, wake up".
10. Push-to-talk while ambient is on: works normally, and ambient doesn't
    double-process the same speech.
11. With ambient on, join a call (Zoom/FaceTime/Meet in a browser) or open
    Voice Memos and record. Within ~2s the widget reads "Paused (mic in
    use)"; it returns to listening when the other app releases the mic.
12. **Terminal (Orca / Terminal / iTerm)**: dictate "Let's test and see what
    happens when we paste in here" — it pastes as that sentence, never a
    command. Dictate "look at recording dot rs and fix it" — `@recording.rs`
    keeps its `@`. Dictate "one fix paste two ship it" — stays a two-line
    list. Check `app.log` shows `llm: refined ... -> ...` for each.

## Pass criteria

Steps 2–12 behave as described. Record the `voice match` scores from
steps 6–7 in this file so the 0.5 threshold can be tuned.
