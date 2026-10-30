# Features

Everything currently built and working, grouped by area. This is the "what
does the app actually do right now" reference — see `PRODUCT_SPEC.md` for
the original vision/pitch, and `ROADMAP.md`/`BACKLOG.md` for what's not
built yet.

## Core dictation loop

- **Push-to-talk global hotkey**, configurable from Settings, persists
  across restarts. Uses Carbon's `RegisterEventHotKey` so it works without
  Accessibility permission — only pasting the result needs that.
- **Double-tap Fn to start/stop recording** — an opt-in alternative to the
  hotkey above, matching macOS's own "press Fn twice" dictation gesture.
  Off by default (needs macOS's Input Monitoring permission, a separate
  grant from Accessibility); runs alongside the hotkey rather than
  replacing it.
- **Local speech-to-text** via whisper.cpp with Metal acceleration.
  Tiny/Base/Small (English) models selectable and downloadable from
  Settings, with per-model download progress.
- **Per-mode model switching** without a reload penalty — `WhisperEngine`
  keeps a small LRU pool (up to 3) of warm-loaded contexts, so a mode rule
  pinned to a different model doesn't repay the multi-second Metal shader
  compile on every recording that hits it.
- **Transcribe while you talk** — during push-to-talk, Silero VAD
  (`live.rs`) cuts the audio at natural pauses (600 ms of quiet after
  1.5 s of speech; 20 s soft / 28 s hard cap) and each chunk is
  transcribed in the background, prompted with the vocabulary and the text
  so far. On stop only the tail is left, so progress covers just that, and
  silence between phrases (padded by 300 ms around speech) is never
  transcribed. Isolated Voice is applied per chunk. Recordings under 3 s,
  a missing VAD model, a failed chunk or no detected speech fall back to
  whole-recording transcription. The running text is sent as
  `live-transcript` and shown in the detailed widget; Settings > Voice has
  a "Transcribe while you talk" toggle (on by default). `app.log` gets a
  `live:` line per dictation (chunks, silence skipped, drain/tail ms,
  stop-to-text ms).
- **Developer vocabulary seeding** — an editable list of dev-jargon terms
  (`kubectl`, `camelCase`, framework names, etc.) fed into Whisper's
  `initial_prompt` to bias recognition toward developer speech.
- **Names from your project** — when the frontmost app is a terminal or an
  editor (VS Code, Cursor, Zed, JetBrains, Sublime), Mouthpiece finds the
  repo you're in (a terminal's shell working directory, or the editor's
  window title / open folder) and reads its names from `git ls-files`,
  recent changes and commits, the branch, `package.json` / `Cargo.toml`
  dependencies and declarations in source files. Everything stays in
  memory, refreshes in the background and never delays a dictation; logs
  carry counts only. The top ~40 names go into Whisper's prompt (before
  your own vocabulary, which keeps priority) and to the LLM. After
  transcription, spoken names in the text become the real ones: "the paste
  text function in the paste file" gives `paste_text` and
  `@src-tauri/src/paste.rs` (a bare path outside terminal agents). Exact
  matches first, then one slip in a long name; ambiguous names (two
  `mod.rs`) and ordinary prose are left as spoken. Off for Casual modes
  and literal shell commands. Settings, Vocabulary has the toggle and the
  detected project.
- **Deliver as paste or copy-only** — the transcript is copied to the
  clipboard and a simulated Cmd+V pastes it at the active cursor by
  default; a "copy only" toggle skips the simulated keystroke entirely
  (and doesn't need Accessibility permission on that path).
- **No lost words without Accessibility** (2026-10-27) — if pasting isn't
  allowed, the transcript is copied (and saved to History) instead of
  failing, and the widget shows "Copied · Allow pasting" ("Allow
  pasting" in Pill and Notch). Clicking it shows macOS's prompt, opens
  the Accessibility list, and watches for the grant ("Pasting is on").
  The setup checklist's fix does the same. The tooltip covers the case
  where Mouthpiece already looks on: ad-hoc signed updates need it
  switched off and on again.

## App-aware modes & LLM refinement

- **Local models only** — Ollama's cloud models ("…-cloud", or reported
  with a remote host) are never listed, downloaded, prewarmed or sent a
  prompt, so LLM cleanup can't leave the Mac. A cloud model already
  saved in settings is refused with a message.

- **Frontmost-app detection** (`NSWorkspace`) picks a formatting Mode
  (Plain / Casual / CLI) based on whichever app was focused when
  recording started, with built-in defaults for common apps (terminals →
  CLI, Slack/Messages/Discord → Casual) and user-defined per-app rules.
- **Rule-based CLI formatting** — natural-language-to-shell-command
  patterns for the CLI mode (e.g. "git commit update readme" →
  `git commit -m "update readme"`).
- **Real LLM refinement** via a locally-running Ollama instance, with a
  per-mode "Refine with LLM" toggle and mode-aware prompts. Casual mode
  defaults this on, since rule-based formatting can't meaningfully do
  "sound casual."
- **In-app Ollama model catalog** (Gemma, Llama, Phi, Qwen, Mistral) with
  one-click pull and download progress, plus autodiscovery of any model
  you'd already pulled outside the app.
- **Last-observed refinement latency** surfaced per model in the LLM
  picker, so you can judge which local models are actually fast enough
  before committing to one (a "thinking" reasoning model can take
  minutes to answer a trivial prompt otherwise).
- **App picker with real icons** — native icon extraction (`NSWorkspace`
  + `NSBitmapImageRep`) for the running-apps picker and mode-rule list,
  plus a curated list of common apps so you don't have to have the target
  app open to add a rule for it.

## Voice commands

- **Syntax & Casing Commands** — say "snake case error response handler"
  and get `error_response_handler` pasted directly. Supports snake,
  SCREAMING_SNAKE/constant, camelCase, PascalCase, kebab-case, and Title
  Case. Pure Rust string transformation, no LLM involved, so it's instant
  and works even if Ollama isn't running.
- **Boilerplate Generation** — say "generate boilerplate for a React
  component called UserCard with name and avatar props" and the request
  goes to the local LLM, with a longer timeout than cleanup refinement and
  automatic markdown-fence stripping (small models often wrap code in
  fences despite being told not to). Falls back to normal dictation if
  Ollama is unreachable, so a down LLM never eats your speech.
- **Append clipboard** — end a dictation with "append clipboard" (or
  "paste clipboard"/"insert clipboard") and whatever's currently on the
  system clipboard gets appended to what you said, so a copied stack
  trace, URL, or code snippet doesn't have to be read aloud. Appended raw,
  after mode formatting/LLM refinement, so it's pasted verbatim.
- **"Send it"** — alias for the trailing "press enter" (same Settings
  toggle), also "submit it". Unlike "press enter" it counts only as the
  whole utterance or right after a comma/period, so "I'll send it
  tomorrow" and "fix the bug and send it" are pasted as written.
- **Attach the error** — end with "attach the error" (or "attach the
  error message", or ", with the error") and the latest error output is
  appended as a fenced code block. Source: the frontmost terminal's
  visible text via Accessibility (Terminal.app, iTerm2), else the
  clipboard if it looks like an error. The last error-looking block
  (rustc/cargo, tsc, Python, Node, Jest, Vitest, Playwright, pytest) is
  picked, capped at 60 lines. Nothing found: pastes your words and shows
  "No error found to attach". Captured text is never logged, only line
  counts.
- **Scratch that / undo that** (whole dictation) — undoes the last
  Mouthpiece paste: Cmd+Z in editors/browsers/chat (within 30s), exactly N
  backspaces in terminals (single-line ASCII paste, within 15s, not
  submitted), never Ctrl+U. If the app changed or it can't be done
  safely, nothing is sent and the widget says "Couldn't undo safely
  here". Mid-sentence "scratch that" is still Backtrack.

## History

- **Local transcript history** (JSONL), with configurable retention
  (7/30/90/365 days) and automatic purge of anything older on launch.
  Only logs transcripts that actually reached you — a failed paste or
  copy doesn't show up as a history entry.
- **Work journal** (opt-in, off by default) — each history entry can get a
  one-line LLM-generated summary in git-commit-subject style ("Refactor
  audio module to drop devices safely during recording"), generated in
  the background after the paste already happened so it never adds
  latency. Skips dictations under 6 words (not worth summarizing) and
  falls back to just the raw transcript if Ollama is unreachable.
- **History lock** (opt-in, Settings → History → Privacy) — Touch ID or
  the Mac password (LocalAuthentication) before History is shown, enforced
  in the backend commands that read or change history, not only the UI.
  Locks again when you leave History, hide Settings, or after 5 minutes;
  turning the lock off needs the same confirmation. It's a screen against
  someone at your unlocked Mac, not encryption: `history.jsonl` and
  `app.log` are owner-only (0600) files.
- **Transcript text out of logs by default** — `app.log` keeps events
  and timings, with each transcript replaced by its length ("<42 chars>").
  "Include transcript text in logs" (Settings → History → Privacy) turns
  it back on for bug reports.
- **Per-entry copy/delete** in the History settings section.
- **Full-text search** — a search box filters the entire retained history
  (not just the most recent 200 entries the list view normally loads), by
  transcript text, journal summary, or app name.
- **Reprocessing** — re-run a past transcript's *text* through a different
  Mode (and optionally the LLM refinement pass) without re-recording.
  Doesn't re-transcribe from audio — the raw recording is a transient temp
  file, never retained — so this reformats the saved text, the same
  formatting/refinement a live dictation goes through. Preview the result,
  then copy it or replace the stored entry with it.
- **Original kept next to the refined text** — when the LLM changed a
  dictation, History stores the text from just before refinement (what
  "paste as-is" would have delivered) and marks the entry "Refined".
  Expanding it shows the original with Copy original / Use original.
  Entries the LLM left alone, and older entries, store nothing extra.
- **Edit an entry** (pencil in History) to fix a word by hand.
- **Learns from your corrections** — word-level fixes from History edits,
  and from "Use original" after refinement changed a word, are counted
  locally (only the changed words, in `corrections.json`). The same
  one-to-three-word substitution seen twice shows a card in Settings >
  Vocabulary ("You've corrected 'post gress' to 'Postgres' 2 times. Add
  it?"). Add puts the term in the vocabulary and saves a whole-word,
  case-aware replacement rule applied to every later transcript before
  formatting (listed under the vocabulary, removable). Casing/punctuation
  changes, pure insertions and large rewrites are ignored, as are common
  words. Toggle: "Learn from my corrections" (on by default).

## Meetings (2026-10-26)

- **Meeting mode** — "Start meeting" in Settings → Meetings or the menu
  bar icon records everything the mic hears until you stop it; the widget
  shows the meeting's red dot, level meter and running time, and its
  button stops the meeting. Dictation, voice enrollment and hands-free
  listening wait until it ends.
- **Transcript split by speaker** — after stopping, sherpa-onnx finds the
  speech segments (bundled pyannote segmentation model, 6 MB), each
  segment gets a voice embedding (the bundled Isolated Voice model), and
  segments are grouped by voice: into the number of people set under
  "People in the meeting", or by similarity when it's left on "Work it
  out". One speaker's back-to-back segments become a turn, each turn is
  transcribed with Whisper, and the speaker matching an enrolled voice is
  labelled "You"; the rest are "Speaker 1", "Speaker 2", … in the order
  they first speak. Progress shows in Settings and in the widget, which
  stays usable for dictation meanwhile.
- **Saved as Markdown** in the app data folder's `meetings/` (owner-only
  files): title, length and speaker count, then one paragraph per turn
  with a timestamp. Settings → Meetings lists them with View, Copy, Open
  (in the default text editor) and Delete, behind the History lock.
- **Crash-safe recording** — audio streams to a 16 kHz WAV, flushed every
  5 seconds, so a quit or crash leaves a recording the list offers to
  Transcribe. The WAV is deleted once the transcript is saved. A
  forgotten meeting stops itself after 4 hours.
- Mic only: meetings in the room and calls on speakers. Calls on
  headphones only include your side (system audio is phase 2, see
  `ROADMAP.md`).

## Widget & Settings UI

- **Discard and undo** — Esc while recording (or the widget's ✕) throws
  the recording away without transcribing; Esc while transcribing or
  refining cancels the paste. Either way it's held for 8 seconds:
  "Discarded · Undo" in the widget, or ⇧Esc, transcribes and pastes it
  after all (a cancelled paste pastes the text that was ready). Esc and
  ⇧Esc are registered only while they mean something, so other apps keep
  them, and not at all if the dictation hotkey itself uses Esc
  (`discard.rs`).
- **Live drafts** — while a phrase is still being spoken, its audio is
  re-transcribed about every 0.7 s with a quick pass (Whisper's encoder
  limited to the clip's length, about 30-45% faster on short clips) and
  shown after the finished text; the phrase's final, full-quality text
  replaces it at the next pause. Drafts only run when the transcriber is
  idle and never after you stop, so they can't delay the paste. In the
  real-speech test, a 12 s run-on sentence shows its first words after
  1.7 s instead of 12.3 s.
- **Voice meter fills its space** — as many bars as fit the widget's
  width, newest on the right, older ones fading, levels square-rooted so
  quiet speech still moves.
- **Seven widget styles** — Minimal, Pill (dot and waveform, text on
  hover), Compact, Compact two-line (adds mic and next-dictation mode),
  Notch (black, docked top-center under the menu bar, widens while
  active and re-centers on every resize), Live caption (your words as you
  speak, then what was pasted) and Detailed. Switch from Settings
  (visual picker), the widget's hover menu (Style), or the menu bar
  icon's "Widget style" submenu; all three stay in sync.

- **Three widget display modes** — Minimal (icon-only recording button),
  Compact (the default status pill), and Detailed (a larger panel with a
  persistent, non-truncating status/error area), switchable from Settings.
- **Live input level meter** while recording — a small animated bar
  visualizer (Compact/Detailed) or a pulsing record dot (Minimal), driven
  by the mic's real-time RMS level, so it's visibly obvious audio is
  actually being picked up rather than trusting a static "recording" dot. Bar height is linear in decibels (-60 dBFS empty, -3 dBFS
  full), so normal speech sits mid-meter and louder or quieter words show.
- **Errors that don't get lost** — Compact mode auto-grows and holds
  error text on screen for 6 seconds instead of truncating it (this used
  to silently swallow the Accessibility-permission error).
- **Draggable widget** that remembers its position across restarts.
- **Settings window** (520px wide) covering device, shortcut, models,
  app modes, LLM, vocabulary, history, an in-app log viewer, and general
  preferences, all in one place.
- **In-app Voice Commands reference** — every cross-cutting trigger
  phrase (casing, punctuation, numbered lists, boilerplate, press enter,
  append clipboard) documented right in Settings with a say/get example,
  so unexpected output ("why did that get pasted in ALL_CAPS") has an
  obvious place to check instead of needing to ask.
- **In-app log viewer** — an in-memory ring buffer capturing what used to
  be terminal-only debug output (device lifecycle, mode/model resolution,
  LLM results), so you can see what happened without launching from a
  terminal.

## System integration

- **Auto-launch at login** — off by default (a privacy-first app
  shouldn't silently add itself as a login item), toggle in Settings.
- **Tray icon reflects recording state** — the glyph is the app icon's
  prompt, wave and cursor as plain strokes (drawn by
  `src-tauri/icons/source/tray-icon.py`), a macOS template image so it
  adapts to light/dark menu bars. While recording it turns grey with a red
  dot (green while always-on listening is awake); grey because colored
  icons aren't recolored by macOS, and black would vanish on a dark bar.
- **Custom app icon** — a terminal prompt flowing into a waveform and a
  text cursor, amber to violet (source in `src-tauri/icons/source/`), for
  Dock/Finder/the About window and the website.
- **Deep-link/CLI automation hooks** — `open mouthpiece://start-recording`
  / `stop-recording` / `toggle-recording` for Raycast, Hammerspoon,
  Alfred, or plain shell scripts. Start/stop are idempotent, so a script
  doesn't need to track recording state itself.
- **Output webhook** — a configurable URL gets a JSON `POST`
  (`timestamp_ms`, `text`, `summary`, `app_name`, `mode`) after every
  delivered dictation, covering Notion/Slack/n8n/Zapier/Make.com/
  webhook.site the way MacWhisper's integrations do, without a bespoke
  client per service. Fires in a background thread so a slow/unreachable
  endpoint never adds latency to the paste, and failures are logged, not
  surfaced. Off by default, plus a "Send test event" button in Settings.
  Complements the deep-link hooks above, which only cover the *input*
  side.
- **Reply to a waiting coding agent by voice** — Claude Code's
  Notification, Stop and UserPromptSubmit hooks run a bundled script
  (`mouthpiece-hook`, kept at `~/Library/Application Support/
  dev.mouthpiece/bin/`) that opens `mouthpiece://agent-waiting?...`
  in the background. Mouthpiece finds the app that owns the agent's
  terminal by walking the hook's parent processes (falling back to the
  exported bundle id, then `TERM_PROGRAM`), and keeps up to 8 waiting
  agents (newest first, one per session, expiring after 30 minutes). The
  widget shows a "Claude is waiting · folder" pill (click to go to that
  window) and plays a soft system sound (toggle in Settings). A separate
  reply shortcut (default ⌃⌥R) records a dictation for the newest waiting
  agent: when ready, Mouthpiece raises the agent's window through the
  Accessibility API, confirms that app is frontmost, pastes, and returns
  focus to the app and window you were in. Ending with "press enter" (or
  "send it" after a pause) asks for Enter, which is pressed only when the
  window title names the agent's terminal device (tty); otherwise the
  selected tab could be a plain shell in the same folder, where Enter
  would run the reply as a command, so the text is pasted and focus stays
  there for you to check and press Return. "Scratch that" as a reply
  cancels it. The installed hook script carries a random per-install
  token (readable only by you) and links without it are ignored, so a web
  page can't pose as a waiting agent. Timings are logged to `app.log`.
  Settings > Coding agents has the hook snippet with a copy button, the
  shortcut, the sound toggle and the waiting list.
  Limits: the window is identified only by title (folder name or tty), so
  Terminal, VS Code and Cursor usually work, while iTerm, Orca and Warp
  often don't name the folder; with several windows and no match, the
  reply is copied to the clipboard instead of guessing. With a single
  window, tabs and split panes can't be selected, so the reply goes to
  whichever is showing (and isn't submitted). It never pastes into a different app. Answering a
  permission prompt in the terminal yourself doesn't clear the pill until
  the next prompt you submit, a Stop, or expiry.

## Reliability

- **Microphone permission** — `src-tauri/Info.plist` carries
  `NSMicrophoneUsageDescription`; without it macOS never prompts and the
  input stream delivers only zeros (the enterprise "mic never works"
  bug, present in every release before 0.3.4). The release workflow fails
  if a built app lacks the key. The setup checklist shows the permission
  state (AVFoundation): "Allow the microphone" shows macOS's prompt,
  "Microphone access is off" opens Privacy & Security > Microphone, and a
  device-management block is named as such. A silent recording or a
  silent stretch mid-recording is blamed on permission when that's the
  cause, otherwise on a muted mic or a zero input level. The
  `com.apple.security.device.audio-input` entitlement is in place for
  when the app is notarized (hardened runtime needs it).

- **Mic device retry+fallback** — retries opening a specific input
  device a few times before falling back to the system default, fixing
  the "stale device handle right after selecting it" failure mode. When the picked mic is missing and the system default is Bluetooth
  headphones, the Mac's own mic is used instead (CoreAudio transport
  type, `input_devices.rs`).
- **A dead mic is swapped mid-recording** — a mic you didn't pick that
  sends pure silence for 2 s is replaced by the next real mic (the Mac's
  own first) in the same recording; its audio is converted to the clip's
  format, so nothing is lost. A silent mic (during or after recording)
  shows **Switch mic**, a menu of mics with the Mac's own first; a pick
  becomes the chosen mic and takes effect mid-recording.
- **Widget menu in every style** — right-click the widget, or click its
  ⌄ button on hover (Pill, Notch, Caption, Detailed), for a native menu
  with the hover panel's choices: microphone (the Mac's own first), mic
  mode, next dictation's mode, widget style, Settings. Native, so it draws
  outside the widget instead of resizing it. Compact and Stacked keep the
  hover panel too.
- **Show the widget only while dictating** (Settings > Dictation >
  General, off by default) — the widget appears when a dictation or
  meeting starts and hides itself once it's over: 0.6 s after a paste,
  after an error or note has had its 6 s, never while the pointer is on
  it, the undo offer is up, or it's asking to allow pasting. The menu
  bar's Show/Hide Widget still brings it up by hand, and it stays until
  the next dictation ends.
- **Warm model pre-load on startup**, so the first real recording doesn't
  pay for Metal shader compilation during the user's first
  "transcribing…".
- **Setup checklist (readiness)** — one set of checks (speech model
  downloaded, paste permission, hotkey registered, microphone, Ollama and
  the models your modes use) shown at the top of Settings with a fix
  button each, and as "Finish setup" on the widget when something blocks
  dictation. Hidden once everything is ready.
- **Survives a taken hotkey** — if another app owns the shortcut, the app
  still starts (it used to fail to launch) and the checklist says so.
- **Single instance** — a second launch hands off to the running copy
  instead of starting another one that fights over the hotkey and mic.
- **Verified model downloads** — pinned to a fixed whisper.cpp revision,
  checked against SHA-256, with stall timeouts and no leftover partial
  files.
- **Refine only when it's needed** (on by default, Settings → LLM) —
  a clean transcript skips the LLM and pastes right away; filler words, a
  stutter or an unpunctuated run-on still go through it. Casual mode and
  modes with custom instructions always refine. Refinement used to add
  3-10s per dictation while mostly returning the text unchanged.
- **Two-stage progress in the widget** — "Transcribing 64%" in the theme
  accent, then "✦ Refining 40%" in violet with a shimmer and the model
  name on hover. Transcription progress comes from Whisper plus the
  model's learned speed on your Mac; refinement progress is exact, from
  Ollama's streamed answer. Minimal mode shows the same as a colored ring.
- **Paste as-is** — click the violet Refining bar, or press the hotkey
  plus Shift (⌘D → ⇧⌘D; plus Option if the hotkey already has Shift), to
  skip LLM cleanup. While recording, the shortcut stops and pastes without
  refining.
- **Your vocabulary reaches the LLM** — refinement is told your terms
  (Firestore, kubectl, project names), so a misheard one comes back
  spelled your way.
- **Bad LLM answers stop early** — a reply that opens with chatter
  ("Sure! Here's…") or runs far longer than what you said is cut off as
  it starts, and the unrefined text pastes.
- **Every speech model a mode uses is warmed at launch**, not just the
  default, so the first dictation in that mode isn't slow.
- **Recording timer** next to the level meter.
- **Faster LLM cleanup** — Ollama keeps the model loaded for 30 minutes,
  and recording start pre-warms every model a mode uses, so the load
  overlaps with you talking instead of being added after.
- **Nothing blocks the UI** — commands that do I/O (Ollama, history,
  insights, device enumeration) run off the main thread, so a slow Ollama
  or a History reprocess no longer freezes the widget, tray and hotkey.
- **Latency you can see** — every dictation logs
  `perf: stt=… llm=… deliver=… total=…`, history records the wait, and
  Insights shows the typical and slowest-1-in-10 time to paste.
- **Crash and log hygiene** — panics on any thread are written to
  `app.log` with a backtrace; `app.log` rotates at 5MB.
- **Failed settings changes are visible** — a save the backend rejects
  shows a toast instead of silently not sticking.
- **Content Security Policy** on the webview, verified by running the
  production build under it in the e2e suite.

## Dictation accuracy (2026-10-19)

Found by running real speech through Whisper in the audio e2e suite:

- **Casing commands work with real Whisper output** — Whisper often writes
  "snake case error response handler" as `snake_case_error_response_handler`
  (and "camel case user ID" as `camelCase user ID`); both are recognized.
- **Lists from real transcripts** — Whisper writes "One, … Two, …" as
  "1. … 2. …" on one line; those become a line-broken list. Numbers and
  ordinals mid-sentence ("one question about the two failing tests") no
  longer turn into a list. List items are capitalized.
- **Backtrack keeps your sentence** — "Let's meet at two, actually three"
  gives "Let's meet at three." (it used to leave only "three."). Longer
  corrections replace just the sentence they interrupt.
- **Spoken "period"/"question mark" capitalize the next sentence**, and
  "comma" misheard as "Kama" still becomes a comma.
- **"…, press enter."** no longer leaves a dangling comma.
- **Terminal file tags include paths** — `src/lib.rs` becomes
  `@src/lib.rs` for terminal coding agents.
