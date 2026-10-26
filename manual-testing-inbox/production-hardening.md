# Production hardening: CSP, single instance, setup checklist, verified downloads

Branch: `main` (built directly on main, 2026-10-19)
Files changed: `tauri.conf.json` (CSP), `lib.rs` (single instance, hotkey
startup, widget position saves), `readiness.rs` (new), `shortcut.rs`,
`models.rs`, `llm.rs`, `logging.rs`, `history.rs`, `insights.rs`,
`recording.rs`, `src/settings/` (Settings split into one file per page),
`src/ui/toast.tsx`, `src/WidgetView.tsx`, `src/App.tsx`

## What to verify

The CSP was checked against a production build in a browser, not inside
Tauri's real webview and IPC. A wrong policy there means a blank window, so
it has to be checked in the built app. The rest touches startup and
permissions, which no automated test exercises.

## Steps

1. `bun run tauri build --debug`, open
   `src-tauri/target/debug/bundle/macos/Mouthpiece.app`. The widget
   renders; open Settings and click through every sidebar page. Nothing is
   blank, and app icons show in Modes.
2. In that build, dictate once. It pastes, so the CSP isn't blocking IPC.
3. **Single instance**: with the dev app running, run
   `open "src-tauri/target/debug/bundle/macos/Mouthpiece.app"` again. No
   second widget or tray icon; `app.log` shows `second launch handed off`.
4. **Taken hotkey**: quit, set your hotkey to one another app owns (or
   edit `config.json` to `{"meta":true,"code":"Space"}`, which Spotlight
   owns), relaunch. The app starts; Settings shows "Pick a different
   hotkey" at the top; the widget says "Finish setup". Pick a new hotkey,
   and the checklist clears without reopening Settings.
5. **Fresh install**: move `~/Library/Application Support/dev.mouthpiece/models`
   and `src-tauri/models` aside, relaunch. Widget: "Finish setup".
   Settings: "Download a speech model" → **Choose a model** jumps to Voice.
   Download Base; `app.log` shows `models: base.en downloaded and verified`,
   and the checklist clears. Put the folders back.
6. **Accessibility**: turn Mouthpiece off in System Settings →
   Accessibility. Within 30s the widget shows "Finish setup"; the
   checklist's **Open System Settings** opens that exact page. Turn it back
   on; it clears when you return to Settings.
7. **LLM prewarm**: with a mode using LLM refinement and Ollama running,
   `ollama stop qwen3.5:4b`, then dictate a long-ish sentence in that
   mode. `app.log` shows `llm: prewarmed qwen3.5:4b` while you're still
   talking, and the `perf:` line's `llm=` is a few hundred ms, not seconds.
8. **No freeze**: Settings → History → reprocess an entry with LLM. While
   it runs, the widget still responds to hover and the hotkey still
   records.
9. **Latency**: after a few dictations, Settings → Insights → "Time to
   paste" shows a number; `app.log` has one `perf:` line per dictation.
10. **Toasts**: make `config.json` read-only (`chmod 444`), change any
    setting. A "Couldn't save …" toast appears. `chmod 644` it back.

## Pass criteria

Steps 1–10 behave as described, especially 1–2: the built app works under
the CSP.
