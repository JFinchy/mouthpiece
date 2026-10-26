# Double-tap Fn to start/stop recording

Branch: `JFinchy/hippocamp`
Files changed: `src-tauri/src/doubletap.rs` (new), `src-tauri/src/config.rs`,
`src-tauri/src/lib.rs`, `src/SettingsView.tsx`. Listener rewritten
2026-10-14 (`doubletap.rs`, `Cargo.toml`).

## What to verify

An opt-in alternative to the push-to-talk hotkey: double-tapping the Fn/
Globe key toggles recording (start on one double-tap, stop on the next),
running alongside the existing shortcut rather than replacing it. This
needs a real keyboard and macOS's Input Monitoring permission — neither
can be exercised by an automated test. Automated coverage
(`cargo test --lib doubletap::`) only checks the double-tap timing window
(400ms) and Fn-press detection as pure functions.

**Why this is Critical now:** the original listener (`rdev::listen`)
crashed the whole app (SIGTRAP) whenever it saw a keystroke on current
macOS, including the synthetic Cmd+V from pasting, so every dictation could
crash with this setting on. Crash report:
`~/Library/Logs/DiagnosticReports/tauri-app-2026-10-14-142109.ips`. It's
now a listen-only CoreGraphics event tap that only sees modifier changes.

## Steps

1. From repo root: `bun run tauri dev`. Look for the mic icon in the menu
   bar (not a window).
2. Open Settings → Dictation, and check **"Double-tap Fn to start/stop
   recording"** under the push-to-talk key button.
3. macOS should prompt for **Input Monitoring** permission the first time
   (System Settings → Privacy & Security → Input Monitoring) — confirm the
   prompt appears and grant it. If it doesn't appear automatically, check
   there manually and add/enable Mouthpiece yourself, then retry. Under
   `tauri dev` the entry is the dev binary, listed as **tauri-app**
   (`src-tauri/target/debug/tauri-app`).
4. Double-tap the Fn/Globe key (bottom-left of most Mac keyboards) quickly,
   twice. Confirm recording starts — widget shows the recording state.
5. Double-tap Fn again. Confirm recording stops and the dictation pastes
   normally, same as a push-to-talk-triggered recording would.
6. Tap Fn once, wait ~1 second, then tap it again (slower than a real
   double-tap). Confirm this does *not* start recording — the 400ms window
   should have expired.
7. Triple-tap Fn quickly (three taps in rapid succession). Confirm this
   reads as one double-tap (starts recording) rather than misfiring twice
   or leaving the third tap dangling — i.e. taps 1+2 pair up and start
   recording, tap 3 begins a fresh (unpaired) window.
8. Use Fn normally for its regular OS purpose (e.g. Fn+Delete for forward
   delete, or an emoji picker/dictation shortcut if configured) while the
   setting is on. Confirm it still works normally — the listener should be
   passive (listen-only) and not interfere.
9. Uncheck the setting. Confirm double-tapping Fn no longer triggers
   anything, while the original push-to-talk hotkey still works.
10. Quit and relaunch the app with the setting left checked. Confirm
    double-tap Fn still works without needing to re-check the box (i.e.
    the listener restarts automatically at launch when it was left on).
11. **Crash regression:** with the setting on, dictate 3–4 times with
    push-to-talk (Cmd+D) into Notes and a terminal. Every one pastes and
    the app stays running (`pgrep -x tauri-app`), with no new
    `tauri-app-*.ips` in `~/Library/Logs/DiagnosticReports`.

## Pass criteria

Steps 3-5 establish the core flow works end-to-end on real hardware;
step 6 confirms the timing window isn't too generous; step 7 confirms
repeated taps don't misfire; step 8 confirms no interference with Fn's
normal OS behavior; steps 9-10 confirm the toggle and persistence both
work correctly; step 11 confirms pasting no longer crashes the app.
