# Roadmap

What's left, in priority order. See `FEATURES.md` for everything already
built, `PRODUCT_SPEC.md` for the product vision, and `BACKLOG.md` for full
detail behind each item here (known issues, research findings, effort
estimates). Shipped work moves out of this file into `FEATURES.md` instead
of lingering here struck through — this file is meant to answer "what's
left," not "what's the history."

## In progress — always-on foundation (agreed 2026-10-05)

Groundwork for always-listening dictation, in build order. Extends
`BACKLOG.md`'s "Ambient/hands-free listening mode" entry; a wake phrase is
required before ambient speech pastes, since auto-pasting every detected
utterance would eventually type a side conversation into an editor.

- [x] Persistent `app.log`, 30s heartbeat, signal + exit logging (to
  diagnose the dev process dying silently)
- [x] Capture state machine (`capture.rs`) replacing the `is_recording`
  flag; every trigger (hotkey, double-tap Fn, tray, deep link) goes
  through one request API, transitions/rejections logged
- [x] Device status: picked mic missing → keep falling back, but surface
  it (event + widget pill), log only on change
- [x] Codebase bug review pass (fix what's found)
- [x] Silero VAD (already in linked `sherpa-onnx`) for ambient speech
  detection
- [x] Pre-roll buffer (1s) + silence endpointing (700ms, 30s cap)
- [x] Ambient mode: speaker-gated (existing voice enrollment), wake
  phrase required before anything pastes, paste guard (same frontmost
  app), green tray dot + widget label, voice sleep/wake, 10-min
  auto-sleep
- [x] Pause while another app is using the mic (calls, incl. browser
  Meet), via CoreAudio's per-process input list; resumes on its own
- [x] Ambient metrics in `app.log` (segments, speaker rejects, wake hits)
- [x] Terminal paste review: CLI-mode LLM no longer synthesizes shell
  commands (root cause of every bad terminal paste), refinement output is
  logged and sanity-checked, file tags survive refinement, line breaks
  survive file tagging, "press enter" waits for async terminal pastes
- [ ] Manual verification: enroll, enable, tune VAD/voice-match thresholds
  from `app.log` metrics (needs real speech, see manual-testing-inbox)

## Production readiness (from `AUDIT.md`, 2026-10-19)

The 2026-10-19 audit pass is done (see `AUDIT.md` and `FEATURES.md`).
Still open, in order:

1. Verify the hardening in the built app (`manual-testing-inbox/production-hardening.md`)
2. Notarization (needs an Apple Developer account)
3. Opt-out for deep links starting a recording
4. Microphone permission in the setup checklist

## Now — the voice interface for coding agents (agreed 2026-10-20)

Positioning: Mouthpiece is the voice interface for coding agents. It
knows your codebase and runs on your Mac. Fast, accurate dictation is
table stakes across the market; talking to agents with real file and
symbol names, locally, is what nobody else does. Everything stays in-app
for now (no repo-committed config yet, see `.devwhisper.json` in
`BACKLOG.md`).

Built and merged on 2026-10-21, shipped in v0.3.0 on 2026-10-22 after a
code review, before the manual pass. Each still has a manual-testing-inbox
file awaiting a real-Mac check:

1. **Codebase-aware vocabulary** — detect the project behind the
   frontmost terminal/editor, collect its file, symbol, dependency and
   branch names, and match spoken forms after transcription ("paste text
   in the paste file" -> `paste_text`, `@src-tauri/src/paste.rs`).
2. **Transcribe while you talk** — VAD-chunked transcription during
   recording, long silences skipped, only the tail left at stop; live
   text in the widget.
3. **History keeps the original + learns from corrections** — refined
   entries keep the pre-refinement text; repeated fixes become suggested
   vocabulary and replacement rules.
4. **Agent voice commands** — "send it", "attach the error" (latest stack
   trace as a code block), "scratch that" (safely undo the last paste).
5. **Reply to a waiting agent** — Claude Code hooks tell Mouthpiece when
   an agent is waiting; a reply hotkey sends your dictation to that
   agent's window and returns focus.

Also now: a download-and-features website (GitHub Pages, `site/`), and the
production items above (notarization above all).

Next after these: snippets polish, selected-text context for refinement,
"look at this" screenshots, structured prompts, voice tune-up (all in
`BACKLOG.md`).

Frozen until manually verified: always-on listening and Isolated Voice
(see `manual-testing-inbox/`). Deprioritized: full Command Mode, the
Local Config Agent, vocabulary coaching, the iOS keyboard.

## In testing

- **Isolated Voice mode** — Settings toggle filtering a recording down to
  the primary user's voice before Whisper transcribes it. Both phases now
  built: phase 1's energy-gate background-noise filtering (merged, tested)
  and phase 2's voice enrollment + speaker-embedding cosine-similarity
  masking (sherpa-onnx + a bundled WeSpeaker model), which can actually
  reject a second person talking, not just quiet noise. Needs manual
  end-to-end verification (real enrollment, background-voice rejection,
  threshold tuning) before it's considered done. See `BACKLOG.md` for the
  full design.

## Next — needs its own design pass before implementation

- **Meeting mode, phase 2: calls on headphones** — phase 1 (mic only,
  transcript split by speaker) is built (2026-10-26, see `FEATURES.md`).
  Phase 2 also records the Mac's own audio output (ScreenCaptureKit,
  Screen Recording permission), kept as a second channel so the mic is
  "You" and system audio is everyone else. About a week. Before it,
  measure phase 1 on a few real recordings. Full design and risks in
  `BACKLOG.md`.

- **The Local Config Agent** — voice-driven edits to `~/.zshrc`, Neovim
  `init.lua`, MCP settings. Needs filesystem *write* access driven by
  voice, so it needs a safety design (confirmation step, diff preview
  before applying) rather than just a new prompt template. Not started —
  deliberately deferred, see `BACKLOG.md`.
- **Stable local codesigning identity** — each dev rebuild produces a
  differently-signed binary, so macOS revokes the Accessibility grant
  every time, which has now caused a real incident (paste silently
  stopped working — see `BACKLOG.md`). Fix is a self-signed certificate
  used consistently for dev builds; touches Keychain trust settings, so
  better done live than unsupervised.

## Later — redesign & research-gated

- **Command Mode** (Talon-inspired) — a second, opt-in always-listening
  mode alongside today's push-to-talk dictation: short spoken phrases
  mapped to actions (keystrokes, shell commands, our own deep-link hooks)
  instead of formatted text. See `COMMAND_MODE.md` for the full
  feature-by-feature breakdown with tasks/subtasks — this is bigger than
  everything else in this file combined, mainly because it needs a new
  continuous-listening/VAD audio pipeline, not an extension of the
  existing explicit start/stop one. Not started.
- **Full Modes UI redesign** to match SuperWhisper: named user-created
  presets (not a fixed Plain/Casual/CLI enum), per-mode shortcuts, a
  categorized app picker, sidebar navigation instead of one long
  scrolling Settings window. The backend groundwork (`app_detect.rs`,
  `modes.rs`, `llm.rs`, `history.rs`) is solid; this is now a UI/data-model
  redesign, not new backend work.
- **Parakeet as an alternative STT model** — researched, not implemented.
  `parakeet-rs` carries the hard parts, but Apple Silicon acceleration is
  unresolved (CoreML flagged unstable by the crate's own maintainer).
  Recommended as a validate-first spike on real hardware before building
  any Settings/download UI for it.

## Future state (from `PRODUCT_SPEC.md` §5, unchanged)

The longer-term vision beyond push-to-talk dictation: continuous
full-duplex voice interaction, a Socratic "Rubber Ducking" mode, and local
TTS for a full conversational loop. Nothing started here yet — everything
above is prerequisite groundwork.
