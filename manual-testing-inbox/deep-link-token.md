# Dictation deep links require the install token

Branch: `main` (uncommitted, security audit 2026-11-05)
Files changed: `src-tauri/src/agents.rs`, `src-tauri/src/lib.rs`,
`src-tauri/src/webhook.rs`, `README.md`, `FEATURES.md`, `.gitignore`,
`.github/workflows/*.yml`

## What to verify

`mouthpiece://start-recording` / `stop-recording` / `toggle-recording` used
to work from any web page, so a site the browser was told to "always
allow" could start the mic and have the transcript pasted into its own
page. They now need `?token=` with this install's token (the same one the
agent hook uses). Needs a human because it's about the real URL handler,
not unit-testable logic. Also checks that webhook failures no longer log
the full webhook URL.

## Steps

1. Build and launch the app (`bun run tauri dev`).
2. In Terminal: `open mouthpiece://start-recording` — nothing should
   happen. `app.log` (Settings → Logs) shows
   `deep-link: ignoring "start-recording" without this install's token`.
3. Run:
   `T=$(cat ~/Library/Application\ Support/dev.mouthpiece/agent-hook-token); open "mouthpiece://start-recording?token=$T"`
   — recording starts. Say a sentence, then
   `open "mouthpiece://stop-recording?token=$T"` — it transcribes and pastes.
4. `open "mouthpiece://toggle-recording?token=wrong"` — ignored, logged.
5. Agent reply still works: trigger a Claude Code Notification hook (see
   `agent-reply.md`) and confirm the waiting-agent pill appears.
6. Settings → Webhook: set `https://example.invalid/hooks/SECRET123`, press
   Send test. The error and `app.log` show `https://example.invalid/…`, never
   `SECRET123`.

## Pass criteria

Tokenless and wrong-token dictation links do nothing (and are logged);
tokened links start/stop dictation as before; agent pills still arrive;
no webhook path/query appears in logs or the test error.
