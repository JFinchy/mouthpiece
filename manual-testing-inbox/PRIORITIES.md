# Manual testing priorities

Work top-down, Critical first. Tier = impact if it turned out to be
broken. On pass, move the line to Backlog (dated); on fail, leave it in
place with a one-line note.

## Critical

- [ ] Rename to Mouthpiece: an updated Dev Whisper keeps its data, hook and login item - see `rename-to-mouthpiece.md`
- [ ] Microphone permission in the built app (prompt, denied, MDM-blocked states) - see `mic-permission.md`
- [ ] Production hardening: CSP in the built app, single instance, setup checklist, verified downloads - see `production-hardening.md`
- [ ] Double-tap Fn (listener rewritten 2026-10-14 after the old one crashed the app on paste) - see `double-tap-fn.md`
- [ ] Always-on listening + shared capture/device groundwork - see `always-on-listening.md` (steps 2-4 and 12 affect every dictation)
- [ ] Voice commands (punctuation, lists, press enter) - see `voice-commands.md`
- [ ] Modes: per-mode model and LLM refinement - see `modes-rework.md`
- [ ] Auto-update from a published release - see `auto-update.md`

## Important

- [ ] Dictation accuracy fixes (casing, lists, backtrack, capitalization) - see `dictation-accuracy.md`
- [ ] Mic visibility, quick switching, "no sound" warnings - see `mic-visibility-and-silence.md`
- [ ] Isolated Voice + voice enrollment (required before always-on) - see `isolated-voice.md`
- [ ] Names from your project (terminal/editor repo names in dictation) - see `project-names.md`
- [ ] Snippet library - see `snippets.md`
- [ ] Backtrack self-correction - see `backtrack.md`
- [ ] History search + reprocessing - see `history-search-reprocess.md`
- [ ] History keeps the original + learning from corrections - see `history-original-and-corrections.md`
- [ ] Output webhook - see `output-webhook.md`
- [ ] Agent prompt voice commands (send it, attach the error, scratch that) - see `agent-prompt-voice-commands.md`
- [ ] Transcribe while you talk (chunked transcription, silence skipping) - see `transcribe-while-talking.md`
- [ ] Reply to a waiting coding agent by voice (hook, pill, reply shortcut, focus hop) - see `agent-reply.md`
- [ ] Privacy: History lock (Touch ID), transcript-free logs, no Ollama cloud models - see `privacy-lock.md`
- [ ] Discard with Esc (and ⇧Esc undo), seven widget styles incl. Notch - see `discard-and-styles.md`

## Minor

- [ ] Compact widget: 280px, contained flyout with thin scroll bar, full error text - see `compact-widget-polish.md`
- [ ] New app icon and menu bar icon (incl. recording dot on dark menu bars) - see `new-icons.md`
- [ ] Live input level meter - see `live-level-meter.md`
- [ ] Insights page - see `insights.md`
- [ ] Signal sidebar Settings, theme picker, widget flyout - see `signal-sidebar-settings.md`
- [ ] Settings layout toggle, About page, vocabulary books - see `settings-layout-toggle-about-vocab-books.md`
- [ ] Settings visual cleanup - see `settings-dictation-cleanup.md`
- [ ] Dictation page order + model speed/accuracy bars - see `dictation-reorder-model-bars.md`

## Backlog (passed)

- [x] 2026-10-14 - Stable dev signing + crash-proof logging - see `dev-signing-crash-logging.md`
- [x] 2026-09-08 - Append clipboard - see `clipboard-append.md`
