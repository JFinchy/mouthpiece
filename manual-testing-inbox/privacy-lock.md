# Privacy: History lock, transcript-free logs, no Ollama cloud models

Branch: `main` (committed directly, 2026-10-22)
Files changed: `src-tauri/src/privacy.rs`, `history.rs`, `logging.rs`,
`llm.rs`, `pipeline.rs`, `recording.rs`, `ambient.rs`, `config.rs`,
`lib.rs`, `src/settings/sections/privacy.tsx`, `history.tsx`, `llm.tsx`

## What to verify

Touch ID / password prompts come from macOS (LocalAuthentication) and
can't run in automated tests; nor can a real Ollama cloud model. The UI
tests cover the lock screen against a fake backend only.

## Steps

1. Install 0.3.2. Settings → History: the Privacy box shows both toggles
   off; History looks as before.
2. Turn on "Lock History with Touch ID or your password". History is
   replaced by "History is locked" right away.
3. Click Unlock History: the macOS prompt says Mouthpiece wants to "show
   your Mouthpiece history". Touch ID (or password) → transcripts show.
   Cancel → stays locked, no error shown.
4. Go to another Settings page and back: locked again. Unlock, then
   hide Settings (close the window) and reopen: locked. Unlock and wait
   5 minutes: locked.
5. While locked, untick the lock toggle: macOS asks to "turn off the
   History lock". Cancel → stays on.
6. Dictate something, then `grep "raw transcript" ~/Library/Application\ Support/dev.mouthpiece/app.log | tail -1`:
   shows `<N chars>`, not your words. Tick "Include transcript text in
   logs", dictate again: the words appear. Untick it.
7. `ls -l ~/Library/Application\ Support/dev.mouthpiece/{history.jsonl,app.log}`:
   both `-rw-------`.
8. With Ollama: `ollama pull gpt-oss:20b-cloud` (needs `ollama signin`),
   or skip if you don't have an Ollama account. Settings → LLM doesn't
   list it. If it was the active model, the yellow warning shows and a
   refined dictation pastes unrefined with `llm: refused cloud model` in
   app.log.

## Pass criteria

History never shows (or returns via Settings) without a successful macOS
authentication while the lock is on; logs carry no dictated text unless
asked; no request reaches a cloud model.
