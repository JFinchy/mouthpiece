# Backlog

Tracking for known issues, bugs, and feature ideas that fall outside the
active Day One MVP scope. See `PRODUCT_SPEC.md` for the product vision and
build phases.

## Known Issues

- **iPhone Microphone (Continuity) unreliable** — intermittently captures
  zero audio frames even though the stream opens/plays/stops cleanly.
  Retry+fallback logic mitigates the "stale device handle" failure mode,
  not the "opens but delivers no data" one. See the README's Known
  Limitations section for the full diagnosis. Low priority — works fine
  with a normal mic, and iPhone-as-mic is a nice-to-have.
- **Dev rebuild breaks Accessibility permission** — each `cargo`/`tauri
  build` produces a differently-signed binary, so macOS treats it as a new
  app and the Accessibility grant (needed for paste) has to be re-granted
  after every rebuild during development. Only affects the local dev
  workflow, not signed release builds. Actually bit a real session
  (2026-08-16): after ~8 rebuilds in one night, paste silently stopped
  working — the error was real (`transcribe_and_paste` correctly emitted
  it) but the widget was too small and truncated to show it, so it looked
  like a silent failure instead of an obvious permission prompt. Fixed the
  *symptom* — the widget now auto-grows and holds error text on screen for
  6s instead of truncating it (see `widget.rs`, WidgetView.tsx) — but the
  root cause (re-signing churn) is still open. Worth a stable local
  codesigning identity now that it's demonstrably annoying enough.
- **"Thinking" LLM models are too slow for refinement by default** — the
  default Ollama model picked up on this machine (`qwen3.5:4b`) is a
  reasoning/"thinking" model that burns many seconds on internal
  chain-of-thought before answering even a trivial prompt (a plain
  `ollama run` "say hi" took minutes). Fixed by passing `"think": false`
  in the `/api/generate` request (`llm.rs`), which Ollama respects for
  models that support toggling it — with that, refinement is ~1-2s. If
  a user's chosen Ollama model doesn't support the `think` flag at all,
  this has no effect and slow models will still be slow. Last-observed
  latency per model is now surfaced in the Settings LLM picker (shipped
  2026-08-15) so users can judge which local models are fast enough.
- ~~**Paste lands as a bare "v" instead of the transcript**~~ (fixed
  2026-08-20) — intermittently, only the letter "v" would land instead of
  a real paste. Two compounding causes, found across a few live-testing
  rounds:
  1. The synthetic Cmd+V in `paste.rs` didn't give the Meta keydown enough
     time to register as "held" before the V keydown followed — the
     frontmost app would see a bare "v" instead of the combo. Fixed by
     adding an explicit 40ms settle between the Meta-down and V-down
     events specifically (not on every event in the sequence).
  2. Starting a recording called the widget window's `set_focus()` (not
     just `show()`), stealing keyboard focus from whatever app the user
     was dictating into — the paste (whenever it happened to actually
     land as a proper Cmd+V) would target the widget instead, needing a
     manual click back into the real target app first. Fixed by dropping
     `set_focus()` from `recording.rs`'s start-of-recording handler —
     `show()` alone is enough for the widget to be visible without
     grabbing focus.
- ~~**Flaky test: `cargo test --lib` concurrent whisper-context
  loading**~~ (fixed 2026-08-15) — `stt::tests::transcribes_without_panicking`
  and `stt::tests::transcribe_with_model_override_switches_contexts` both
  load a real ggml model onto Metal; cargo test runs tests in parallel by
  default, and running both at once reproduced a real (not just
  app-contention-related) `"Failed to create a new whisper context"`
  failure. Fixed by serializing the two tests with a shared
  `WHISPER_CONTEXT_LOAD_LOCK` mutex in `stt.rs`'s test module — 5/5 clean
  runs after the fix, 1 failure in 2 runs before it.

## Feature Requests

- **Insights section** (shipped 2026-08-18, loosely modeled on Wispr
  Flow's "Your usage" tab, adapted for a local single-user app) — a new
  Settings page (`InsightsSection` in `src/settings/sections/insights.tsx`) with total
  words dictated, a "smart formatting applied" count (punctuation/lists/
  casing/snippets/Backtrack), words-per-minute, per-app usage breakdown,
  and a streak calendar heatmap. Two of Flow's cards don't have an honest
  local equivalent and were deliberately adapted rather than copied:
  - WPM drops Flow's "Top 0.1%" population percentile (no userbase to
    rank against locally) in favor of a real per-device average compared
    against this device's own personal best, shown as a semicircle gauge
    (arc length is exact for a fixed-radius sweep, not approximated).
  - "Fixes made by Flow" (their cloud-LLM correction count) becomes
    "smart formatting applied" — a count of dictations where a
    deterministic pass actually fired, not implying an LLM judged
    anything.
  - New, not in Flow's version: a **feature-adoption checklist**
    ("Getting the most out of Mouthpiece") scoring how many of six
    trackable features (Vocabulary customization, Snippets, punctuation
    commands, Backtrack, casing commands, per-app Modes) have actually
    been *used*, not just configured — each unused one gets a one-line
    suggestion. Most of the app's power is opt-in dictation-time
    commands a user can easily forget exist; this surfaces the gap.
  - New `insights.rs`: pure aggregation over `Vec<HistoryEntry>` +
    `AppConfig`, no Tauri dependency in the core `compute()` function —
    same shape as `punctuation.rs`/`snippets.rs`, fully unit tested
    (streak math, WPM, adoption scoring, calendar-day conversion)
    without a real app handle. Calendar-day labeling for the heatmap uses
    a hand-rolled `civil_from_days` (Howard Hinnant's algorithm, pure
    integer math) rather than pulling in `chrono`/`time` as a new direct
    dependency for one feature. UTC-bucketed, not local-midnight — Rust's
    std doesn't know the local UTC offset without an extra crate, so a
    streak can occasionally be off by one dictation right at a day
    boundary; accepted as close enough for a streak indicator.
  - `HistoryEntry` gained three new fields to make this possible:
    `duration_ms` (read from the wav header via new `stt::wav_duration_ms`,
    a cheap header-only read — not full sample decoding), `features_used`
    (which of the pass-through deterministic passes — lists/punctuation/
    backtrack/press_enter — actually changed the text, tracked via
    before/after comparison in `recording.rs`), and `spoken_words`
    (word count of what was actually *said*, captured before snippet/
    casing/boilerplate substitution — deliberately not derived from the
    delivered `text` field, since a two-word snippet trigger expanding to
    a multi-line checklist would otherwise wildly inflate WPM). All three
    are `#[serde(default)]`/`Option`, with fallbacks for pre-existing
    history entries that predate them.
  - Not visually verified in the running app — no native macOS window
    automation available in this session (only browser automation
    tooling), so this shipped on `cargo test`/`tsc --noEmit` passing
    clean rather than an eyes-on check in `bun run tauri dev`. Worth a
    manual look before calling it fully done.

- **Isolated Voice mode** (phase 2 built, in testing, 2026-08-17) — a
  Settings toggle that filters a recording down to the primary user's
  voice before Whisper transcribes it. Runs post-capture (once, on the
  fully-recorded buffer, right before transcription) rather than
  live/streaming, matching the app's push-to-talk architecture and
  avoiding the latency floor a continuous-inference approach would need.
  New `isolate.rs`: an RMS energy gate with hysteresis (separate
  enter/exit thresholds so it doesn't chatter at the boundary) and
  ~150ms hangover padding, zero-fills everything outside the detected
  voiced ranges rather than trimming — keeps the buffer's timeline
  continuous for whisper.cpp and lets a fully-masked clip degrade into
  the existing "no speech detected" path. `stt.rs`'s
  `transcribe_with_model` was split into sample-loading +
  `transcribe_samples()` to give this a hook point between the two.
  - **Phase 1 (merged, energy-gate only)**: 7 unit tests
    (silence/tone/tone-silence-tone boundary cases, exact-zeroing +
    length-preservation for masking).
  - **Phase 2 (built 2026-08-17)**: voice enrollment + speaker-embedding
    cosine-similarity masking, auto-selected over the energy gate once a
    voice is enrolled. Added `sherpa-onnx = "1.13.5"` (official k2-fsa
    Rust crate, Apache-2.0, statically linked — confirmed it resolves
    and builds clean, no dylib bundling/signing risk unlike a raw `ort`
    dependency) plus a bundled `wespeaker_en_voxceleb_resnet34_LM.onnx`
    (25.3MB, Apache-2.0, downloaded from the k2-fsa/sherpa-onnx GitHub
    release and confirmed live) speaker-embedding model as a Tauri
    `resources` entry, CPU inference (no CoreML — cheap enough on CPU
    that chasing CoreML's flagged instability, per the Parakeet entry
    below, isn't worth it here). New `voice_isolation.rs`:
    `start_voice_enrollment`/`stop_voice_enrollment`/
    `get_voice_enrollment_status` commands, reusing the same
    `AudioHandle` dictation uses, guarded by a new
    `RecordingPurpose` mutex on `RecordingState` so a hotkey press
    mid-enrollment can't misfire `transcribe_and_paste` on the
    enrollment clip. Enrollment requires ≥3s of actual detected speech
    (via the phase-1 energy gate) before it'll compute an embedding;
    persists to `voice_profile.json` (own file, mirrors `history.rs`'s
    pattern), tagged with a `model_id` so a future embedding-model swap
    can force re-enrollment instead of silently comparing incompatible
    vectors. `isolate::apply`'s embedding path scores each voiced range
    (widening short ones with context first) via cosine similarity
    against the enrolled profile (starting threshold 0.5, needs
    empirical tuning during manual verification) and fails open — a
    scoring hiccup keeps the segment rather than silently dropping real
    speech. Settings UI now shows enrollment status + Enroll/Re-enroll/
    Stop controls, reusing `ModelsSection`'s `listen()` event pattern.
    9 new unit tests (cosine-similarity accept/reject cases,
    `widen_for_scoring` bounds) plus one gated real-model smoke test
    (loads the actual bundled `.onnx`, asserts a 256-dim embedding on a
    synthetic tone — passed). Full suite: 86 tests green, `tsc --noEmit`
    and `bun run build` both clean.
    Reference for the approach: [whisper-diarization](https://github.com/MahmoudAshraf97/whisper-diarization)
    (pyannote + faster-whisper pipeline) — Python/cloud-model stack we
    won't adopt directly, but useful as a worked example of the
    embedding-then-cluster mechanics if the cosine-similarity masking
    design above hits a snag.
  - **Not yet done**: manual end-to-end verification — needs a live mic
    and a second real/played voice, not something automated tests alone
    can check. See `manual-testing-inbox/isolated-voice.md` for the
    checklist.
  - **Scope note**: this deliberately revisits the 2026-08-16 SuperWhisper
    research's call to leave speaker diarization/meeting-notes out of
    scope. Multi-speaker "meeting mode" (labeling Speaker 1/2/3 segments)
    is still explicitly not part of this work — single-speaker isolation
    only. Meeting mode would need its own diarization-model spike as a
    separate future effort.

- **Output-side workflow automation** (in testing 2026-08-17, from the
  2026-08-17 competitive research — MacWhisper routes transcripts to
  Notion/Zapier/Obsidian/n8n/Make.com/webhooks). Implemented, pending the
  manual pass in `manual-testing-inbox/output-webhook.md`. Design as
  implemented:
  - A single generic primitive rather than N bespoke integrations: a
    configurable **webhook URL** (`webhook_url: Option<String>` in
    `AppConfig`) that gets a `POST` with a JSON body
    (`{timestamp_ms, text, summary, app_name, mode}`) after every
    successfully delivered dictation. This is almost certainly what
    MacWhisper's "integrations" actually are under the hood too — Notion/
    Slack/n8n/Zapier/Make.com all accept incoming webhooks natively, so
    one primitive covers all of them without us maintaining N OAuth
    flows and API clients.
  - New `webhook.rs`: `send_entry(app, entry)` — fires in its own
    background thread (same pattern as journal summarization in
    `recording.rs`) so a slow or unreachable endpoint never adds latency
    to the paste; `ureq::post` with a short (~10s) timeout; failures go
    through `applog!`, never surfaced to the user as an error, since a
    webhook is a side channel, not the primary delivery path.
  - Settings: a URL field plus a "Send test event" button (so a user can
    confirm their Zapier/n8n/webhook.site endpoint is wired correctly
    without waiting for a real dictation to test it).
  - Off by default (empty URL = disabled) — matches the app's existing
    pattern of opt-in background activity (`copy_only`, autostart,
    `journal_enabled`), and deliberately so here: enabling this means
    dictated text leaves the device to a user-chosen destination, which
    cuts against the local-only default and should be a conscious,
    explicit choice, not an accident.
  - Known v1 gap: since the webhook fires immediately after paste but
    journal summaries are generated asynchronously afterward, the
    `summary` field will be `null` at send time when `journal_enabled`
    is on. Acceptable for a first pass — downstream automations that
    care about the summary can fetch/poll `list_history_entries` (once
    that's exposed beyond Tauri's IPC, which it currently isn't) or this
    can be revisited if it turns out to matter in practice.

- ~~**Snippet library**~~ (shipped 2026-08-17, from the 2026-08-17
  competitive research — Wispr Flow: a spoken cue expands to a saved
  block of text, e.g. "PR Checklist" or "Environment Setup"). Distinct
  from the vocabulary editor, which is about recognition *accuracy*, not
  insertion. `snippets.rs`: `Snippet { trigger, body }`,
  `try_expand(text, snippets) -> Option<String>` — case-insensitive match
  of the *entire* trimmed transcript against a configured trigger (not a
  prefix match like casing commands, since a trigger like "standup
  update" is meant to be spoken as a complete, deliberate cue, not a
  directive with content trailing it), tolerant of Whisper's own trailing
  sentence punctuation. Ships with four ready-to-use dev defaults (PR
  checklist, standup update, bug report template, commit message
  template) via `default_snippets()` rather than an empty list, same
  reasoning as `stt::default_vocabulary()` — useful out of the box.
  Wired into `recording.rs`'s `transcribe_and_paste` **before** the
  casing-command check — a snippet match is the most explicit,
  intentional signal of the pre-LLM checks (a literal saved macro the
  user, or a shipped default, defined), so it wins over a coincidental
  overlap with a casing directive or boilerplate phrase. Skips mode
  formatting and LLM refinement entirely, same reasoning as casing
  commands: the output is already fully resolved. Logged to history
  tagged with a `"snippet"` mode label (matching the existing
  `"casing"`/`"boilerplate"` label convention). Settings gets a new
  Snippets section (`SnippetsSection` in `src/settings/sections/vocabulary.tsx`) with
  click-to-edit-in-place and delete, persisted via a single
  full-list-replace `set_snippets` command (simpler than a keyed
  add/update/remove trio, since a snippet's trigger — its only natural
  key — is itself user-editable).

- **Smart Formatting / Backtrack parity** (from the 2026-08-17 Wispr Flow
  docs review —
  [Smart Formatting & Backtrack](https://docs.wisprflow.ai/articles/5373093536-how-do-i-use-smart-formatting-and-backtrack)).
  All four deterministic sub-features shipped 2026-08-17 (named
  punctuation, spoken lists, "press enter", Backtrack). Wispr Flow's
  version is a cloud-processed feature; the parts worth building here
  were the ones that work as pure deterministic passes (same shape as
  the already-shipped Syntax & Casing Commands in `syntax.rs`), so
  they're instant and don't depend on Ollama being up. What's left is
  explicitly deferred or out of scope, see below:
  - ~~**Named punctuation commands**~~ (shipped 2026-08-17) — say
    "period", "comma", "open paren", "em dash", "new line", etc. and get
    the literal character instead of the spoken word. `punctuation.rs`:
    `expand_punctuation(text: &str) -> String`, a word-boundary
    find/replace pass over a fixed table covering Wispr's list except
    angle brackets (ambiguous open/close) and bare "at" (too risky a
    false-positive as a common word). Wired into `recording.rs` in the
    same pre-LLM stage as casing commands, ahead of everything else.
    Known rough edge: a leading symbol like `~` doesn't get a space
    inserted before it when it follows a plain word (context this
    deterministic pass doesn't have) — documented in a test, not silently
    wrong.
  - ~~**Spoken numbered lists**~~ (shipped 2026-08-17) — "one... two..."
    or "first... second..." becomes a real newline-separated numbered
    list (`1. ... 2. ...`). Lives in `punctuation.rs` as
    `expand_lists`, alongside the punctuation table, and runs first in
    the pipeline (before `expand_punctuation`) so a command word like
    "period" said right at a list break is still recognized on its own
    rather than glued to the next item's marker — required teaching
    `expand_punctuation`'s tokenizer to treat an embedded `\n` as its
    own hard-delimited token rather than a whitespace separator that
    silently drops it. Requires at least two markers in strictly
    consecutive order starting at one/first (a lone "one" is too common
    a word to treat as a list start) and doesn't capitalize item text
    or add a lead-in colon the way Wispr's context-aware version does.
    Known, accepted false positive: an ordinary sentence using both
    "one" and "two" as plain numbers ("one item for two dollars") also
    matches — no surrounding-context signal available to rule that out
    without an LLM in the loop.
  - ~~**"Press enter"**~~ (shipped 2026-08-17) — a trailing "press enter"
    (tolerant of Whisper's own trailing sentence punctuation) is stripped
    by `punctuation::extract_press_enter` before casing/boilerplate/mode
    ever see it, and a simulated Enter keystroke (`paste::press_enter`,
    reusing the same `rdev` path as the Cmd+V paste) fires after
    delivery. Gated behind a new `press_enter_enabled` Settings toggle,
    default off (an unexpected Enter is a worse failure mode than an
    unexpected paste) and auto-disabled/greyed-out whenever "copy only"
    is on, since copy-only's whole point is opting out of synthetic
    keystrokes. Skipped replicating Wispr's "first-use, ask before
    enabling" discovery-prompt UX — a plain toggle is enough here. If the
    entire utterance is just "press enter", nothing is pasted/copied
    (avoids clobbering the clipboard with an empty string) but Enter
    still fires and no history entry is logged.
  - ~~**Backtrack (trigger-word case only)**~~ (shipped 2026-08-17) —
    `backtrack.rs`: `try_backtrack(text: &str) -> String`, a deterministic
    pass that collapses "X, actually Y" -> "Y" and "X scratch that Y" ->
    "Y". Runs after `expand_punctuation` in `recording.rs`, since the
    "actually" trigger needs a literal preceding comma to fire — bare
    "actually" is far too common a word in ordinary speech ("I actually
    enjoyed it") to treat as a correction cue on its own; requiring the
    comma (a real spoken pause, whether from an explicit "comma" command
    or Whisper's own punctuation) cuts most of that false-positive rate.
    Known residual risk: a hedge like "well, actually, I think it's fine"
    still has the comma and will still misfire — not eliminated, just
    reduced. "scratch that" needs no such gate, it's unambiguous on its
    own. Deliberately narrower than Wispr's version in another way too:
    on a match it discards the *entire* prefix rather than doing Wispr's
    partial word-level diff (their own "at 2 actually 3" example keeps
    "at" and only swaps the number; ours produces just "3"). The
    no-trigger-word natural-restatement case Wispr also catches (via
    full-context LLM judgment) stays out of scope, same as before —
    already partially covered by the existing "fix filler words, false
    starts" instruction in `llm.rs`'s refinement prompts when a mode has
    LLM refinement on (Plain mode defaults it off).
  - **Explicitly deferred, lower priority**: trailing-period-by-app +
    "Writing Style" tuning, and context-aware mid-sentence
    lowercasing/spacing — both cosmetic relative to the above, and the
    trailing-period one requires the same per-app messaging-app
    detection list Wispr maintains, which is a lot of surface for a
    minor casing nicety.
  - **Explicitly out of scope**: file tagging in Cursor/Windsurf itself
    (from the same Wispr page) — those are windowed editors we have no
    hook into (no project file index, no UI-native reference chip the
    way Wispr can insert), unlike the terminal-agent version below.
    Belongs with the selected-text/on-screen context work already
    tracked above under the SuperWhisper gap analysis if ever picked up.

- ~~**File tagging for terminal coding agents**~~ (shipped 2026-08-17,
  requested directly rather than sourced from a competitor) —
  `file_tagging.rs`: `tag_file_references(text: &str) -> String`, tags
  any bare-filename-shaped token (identifier + `.` + a whitelisted
  extension) with a leading `@`, wired into `modes::format_as_cli`'s
  fallback branch (CLI mode, and only for text that didn't already match
  a literal shell directive like `git commit` — tagging inside an actual
  commit message would corrupt it). Works for Claude Code, OpenCode, and
  Gemini CLI without needing our own project file index, since all three
  already parse a literal `@path` typed into their prompt and do their
  own fuzzy file resolution from there — unlike the Cursor/Windsurf case
  above, no app-level hook needed. Known limits: bare filenames only, no
  paths (a spoken "src slash lib dot rs" doesn't reliably glue into one
  taggable token through the existing punctuation pipeline, which has no
  "dot" command); and no project awareness, so it tags anything
  file-*shaped*, real or not — same as if you'd mistyped an `@mention`
  by hand.

- **History reprocessing + full-text search** (in testing 2026-08-17, from
  the 2026-08-16 SuperWhisper gap analysis) — history was append-only with
  no way to search it or re-run a past entry through a different mode.
  - **Full-text search**: `history::search_history_entries` — a linear
    case-insensitive substring scan across transcript text, journal
    summary, and app name, over the *entire* retained history (not just
    the 200-entry cap `list_history_entries` normally returns). No real
    index — history is bounded by the retention window (365 days max), so
    even a busy user's file is at most a few thousand short lines; an
    index would be solving a problem this app doesn't have yet. Wired into
    the History section's existing list via a search box: empty query
    falls back to the normal recent-first `list_history_entries` view.
  - **Reprocessing**: `history::reprocess_history_text` re-runs a past
    entry's *text* through `modes::apply_mode` for a different Mode, and
    optionally `llm::refine`. Deliberately does **not** re-transcribe from
    audio — the raw recording is a transient temp file
    (`audio::write_wav`), never retained past the original transcription,
    so persisting audio long-term would be a real privacy/storage-scope
    decision this entry isn't taking on. The Settings UI shows the
    reprocessed result as a preview (mode picker + "Refine with LLM"
    checkbox + Run), with Copy and Replace actions —
    `history::update_history_entry_text` persists a Replace, clearing the
    old journal summary since it described text that's no longer there.

- **Live input level meter** (in testing, 2026-08-19, from direct user
  request — most other dictation apps show some kind of live waveform/
  level indicator while listening, and the widget's static red dot didn't
  make it obvious audio was actually being captured).
  - `audio.rs`: each format's cpal callback (F32/I16/U16) now computes an
    RMS level of the chunk it just received via a new `rms_level()` and
    stores it into a new `AudioHandle.level: Arc<AtomicU32>` (bit-packed,
    no stable `AtomicF32`). Reset to 0 on stop so the meter doesn't hold
    the last reading once idle. Scaling went through two rounds of live
    tuning: a straight linear gain (4x, then 8x) couldn't satisfy both
    ends — high enough for normal speech to clearly move the meter meant
    loud speech clipped to 1.0 almost immediately, low enough to avoid
    that meant normal speech barely left the floor. Settled on a
    sqrt-compressed (perceptual) curve — `(rms * 15.0).clamp(0.0,
    1.0).sqrt()` — which boosts quiet/moderate input far more than loud
    input, so normal speech shows clear movement immediately and only
    genuinely loud speech saturates to a full "every bar lit" 1.0.
  - `recording.rs`: `toggle_recording`'s start branch spawns a thread that
    polls `AudioHandle::current_level()` and emits an `audio-level` event
    every 50ms (20fps) while `is_recording` stays true, exiting on its own
    once it flips false. Deliberately decoupled from the audio callback's
    own cadence (which fires far more often than any UI redraw needs)
    rather than emitting an event per callback.
  - `WidgetView.tsx`: a small `LevelMeter` (7 bars, most recent samples
    oldest-first) replaces the status text in Compact/Detailed mode while
    recording; Minimal mode (no room for bars in a 46x46 icon) pulses the
    existing record dot's scale off the latest sample instead.
  - 5 unit tests on `rms_level` (silence, empty slice, clamping, relative
    ordering) as a pure function — the polling thread and event emission
    aren't unit tested, same reasoning as other Tauri-command-adjacent
    wiring in this codebase (webhook delivery, history persistence).
  - Verified live against a real mic across several rounds (2026-08-20) —
    confirmed reacting to speech, then retuned twice (linear 4x -> 8x,
    then switched to the sqrt curve above) after feedback that it still
    needed to be much louder before it read as clearly visible. See
    `manual-testing-inbox/live-level-meter.md` for the remaining checklist
    (dynamic range across widget modes, idle reset, repeated start/stop).

- **Double-tap Fn to start/stop recording** (in testing, 2026-08-18, from
  direct user request) — an opt-in alternative trigger alongside the
  existing push-to-talk hotkey, not a replacement. Toggling recording
  (start on one double-tap, stop on the next) rather than
  double-tap-and-hold, matching macOS's own "press Fn twice" dictation
  gesture and how Raycast/Alfred-style double-tap hotkeys behave.
  - **Why a new capture mechanism was needed**: the existing shortcut
    (`shortcut.rs`) goes through `tauri-plugin-global-shortcut` (Carbon's
    `RegisterEventHotKey`), which requires a real modifier (Cmd/Ctrl/Alt/
    Shift) + key code and fires one press/release for a single registered
    combo — it can't bind to a bare modifier key like Fn, and has no way
    to detect a tap-tap pattern. Fn key state only shows up through raw
    global input monitoring. New `doubletap.rs` uses `rdev::listen()` for
    this (already a dependency — `paste.rs` uses `rdev::simulate()` for
    the synthetic paste keystroke) rather than adding a new crate.
    Listen-only on macOS (`CGEventTapOptionListenOnly`), so it observes
    without consuming — Fn's normal OS behavior is unaffected — but needs
    Input Monitoring permission, a separate TCC grant from the
    Accessibility permission `paste.rs` needs.
  - **Listener lifecycle**: `rdev::listen` blocks forever with no clean
    shutdown API, so rather than starting/stopping an OS-level tap on
    every Settings toggle, the background thread spawns at most once per
    process (`std::sync::Once`) — lazily, the first time the feature is
    turned on, or eagerly at launch if it was already on. Disabling the
    feature just flips an `AtomicBool` the callback checks, making it a
    no-op rather than tearing down the tap.
  - 4 unit tests on the double-tap timing window (400ms) as a pure
    function, extracted from the `rdev` callback the same way
    `history::entry_matches` was pulled out of `search_history_entries` —
    testable without a real event loop or `AppHandle`.
  - **Not yet done**: manual verification on real hardware — `rdev`'s Fn
    key mapping and the Input Monitoring permission prompt can't be
    exercised by an automated test. See
    `manual-testing-inbox/double-tap-fn.md`.

- **App-aware modes + LLM refinement** (shipped 2026-08-15) —
  frontmost-app detection (`app_detect.rs`, `NSWorkspace`), a mode
  framework with built-in defaults for common apps (`modes.rs`), a
  Settings UI to add/edit per-app rules including a live running-apps
  picker (no need to switch to the target app first), rule-based CLI
  formatting (a few illustrative natural-language -> shell-command
  patterns), and real LLM refinement via a locally-running Ollama
  instance (`llm.rs`) with a per-mode "Refine with LLM" toggle and a
  "Refining…" UI state. Verified end-to-end against a live Ollama
  instance, not just built blind — see the Known Issues note below on
  "thinking" models for a real gotcha this surfaced.
  - **Not yet wired**: `AppModeRule.stt_model` (per-mode Whisper model
    override) is in the data model and Settings UI, but doesn't
    actually switch `WhisperEngine`'s active model yet. Naively calling
    `set_model()` on every recording that hits a different-model rule
    would reload the whisper context and repay the multi-second Metal
    shader compile each time (see the "pre-warm" work from earlier in
    this project) — needs a small pool of warm contexts (e.g. an LRU of
    2-3 loaded models) rather than a blind wire-up.
  - Casual mode's rule-based path is still a no-op (regex can't
    meaningfully do "sound casual") — now that LLM refinement exists,
    Casual mode should default `use_llm_refinement` to true instead of
    leaving it manual.

- **Redesign Modes UI to match SuperWhisper** — SuperWhisper's mode system
  (see screenshots shared 2026-08-15) is still meaningfully more complete
  than what we have, even after the 2026-08-15 additions above:
  - Modes are **named, user-created presets** ("Voice to text", "Default"),
    not fixed built-ins — ours is still "each app maps to one Mode enum
    value" (Plain/Casual/Cli), not arbitrary named presets with their own
    Language/Voice Model/keyboard shortcut/"Advanced settings".
  - "Activate for apps" uses a proper categorized picker (Mail,
    Messaging, AI chat, Text editing, Coding, Terminal, Browsers, Social
    media, Design) with app icons — we now have a running-apps picker
    (shipped 2026-08-15) but it's a flat list, not categorized/iconed.
  - A left sidebar nav (Home, Modes, Vocabulary, Configuration, Sound,
    Models library, History) — we now have Vocabulary and History as
    their own Settings sections (shipped 2026-08-15), just not as a
    proper sidebar-navigated app, everything's stacked in one scrolling
    Settings window.
  - Still the right call to defer the full redesign: the underlying
    pieces (`app_detect.rs`, `modes.rs`, `llm.rs`, `history.rs`) are
    solid groundwork; this is now purely a UI/IA redesign, not new
    backend work.

- **Transcript history** (shipped 2026-08-15) — local JSONL store
  (`history.rs`), a scrollable History section in Settings, and
  user-configurable retention (7/30/90/365 days) with auto-purge on
  startup. Only logs transcripts that actually got pasted (a failed
  paste doesn't show up as a history entry). Defaults to 30 days, not
  "forever," since dictated text can contain sensitive content.

- **Vocabulary editor** (shipped 2026-08-15) — replaced the hardcoded
  `DEV_VOCAB_PROMPT` constant with a user-editable term list, persisted
  in config and fed into Whisper's `initial_prompt` at transcribe time.

- **Review SuperWhisper's feature set** (researched 2026-08-16) — full
  gap analysis against superwhisper.com's homepage + changelog. Headline
  finding: SuperWhisper is *hybrid local+cloud* (GPT-5/Claude/Gemini/Grok
  as optional refinement backends, Cohere/ElevenLabs as optional cloud
  STT) — it is not local-only, which validates rather than undercuts Dev
  Whisper's privacy-first positioning; cloud-model parity is explicitly
  not a goal here.
  - **Worth adopting, ranked:**
    1. Direct coding-agent integration (Claude Code/Cursor/etc. plugins,
       a coding-agent panel, hook support) — SuperWhisper is leaning into
       exactly Mouthpiece's developer-specific angle; we're better
       positioned to do it locally.
    2. Realtime streaming transcript display (partial results while still
       speaking, not just after release) — real UX upgrade to the
       shipped push-to-talk loop.
    3. ~~Deep-link/CLI hooks to start/stop recording~~ (shipped
       2026-08-16) — `open mouthpiece://start-recording` /
       `stop-recording` / `toggle-recording`, idempotent start/stop, see
       ROADMAP.md.
    4. Selected-text/on-screen context feeding LLM refinement — extends
       the shipped app-aware refinement and boilerplate generation with
       real code context, not just app identity.
    5. ~~History reprocessing + full-text search~~ (shipped 2026-08-17) —
       see the "History reprocessing + full-text search" Feature Request
       entry below for the design and what it does/doesn't cover.
  - **Explicitly skipped as out of scope**: cloud/BYOK AI models
    (contradicts local-only premise), cross-platform support (macOS-only
    by design), enterprise/SOC2/billing features, speaker diarization +
    meeting-notes mode (meeting-assistant territory, not core dictation),
    stats-sharing/growth gimmicks, bulk file/video transcription,
    100+-language translation, theme/cosmetic polish.
  - Filler-word removal, vocabulary CSV import, and mouse-button
    push-to-talk triggers are noted as small, low-priority QoL items if
    ever picked up.

- **Parakeet as an alternative STT model** (researched 2026-08-15) —
  NVIDIA's Parakeet ASR as a second model option alongside Whisper.
  Findings from a dedicated research pass:
  - NVIDIA only ships Parakeet in NeMo/PyTorch format — no official ONNX
    export. But the community has already done the hard part:
    [`istupakov/parakeet-tdt-0.6b-v3-onnx`](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx)
    on Hugging Face has working ONNX exports (incl. int8-quantized), and
    [`parakeet-rs`](https://github.com/altunenes/parakeet-rs) (crates.io,
    383 GitHub stars, 63k+ downloads, actively maintained) is a real Rust
    crate wrapping `ort` (ONNX Runtime bindings) that already implements
    Parakeet's feature extraction and RNNT/TDT decoding — the parts that
    would otherwise be the expensive part of a second backend.
  - Apple Silicon acceleration is the open question: the `parakeet-rs`
    maintainer's own README flags CoreML as *unstable* for this model
    graph and recommends the WebGPU execution provider (Metal-backed,
    but newer/less proven than whisper.cpp's direct Metal path) or CPU.
    One anecdotal claim that CPU alone beats whisper.cpp's Metal path on
    an M3 — plausible given Parakeet's architecture, but needs our own
    benchmark before it's a decision input, not a data point from one
    person's machine.
  - Effort verdict: this is a real second inference backend (different
    feature extraction, tokenizer, decoding — not just "download another
    GGUF"), plus a second native runtime dependency (ONNX Runtime dylibs
    need bundling/signing/notarizing in the `.app`, unlike whisper.cpp's
    static lib) and multi-file model management (encoder + decoder_joint
    + vocab, hundreds of MB–2GB). But it's *not* build-from-scratch —
    `parakeet-rs` carries the hard parts, meaningfully lowering the
    bar versus rolling our own NeMo/ONNX integration.
  - Risks: single-maintainer crate (supply-chain/longevity), CoreML
    instability means the "accelerated" story needs reframing to
    WebGPU/CPU, and third-party ONNX conversions have to track NVIDIA's
    upstream churn (a v2→v3 model revision already happened).
  - **Validate-first spike run (2026-08-21)**, on this M1 Pro dev
    machine: standalone Cargo project (not merged into the app) using
    `parakeet-rs` 0.3.7 against `istupakov/parakeet-tdt-0.6b-v3-onnx`
    (int8, CPU execution provider), benchmarked head-to-head against our
    shipped `whisper-rs`/Metal `base.en` on the same 12s synthesized
    speech clip (macOS `say`, so a known ground-truth transcript):
    - **Speed: Parakeet lost, on this hardware.** whisper.cpp
      `base.en`/Metal: 113ms model load, ~275–320ms inference (~40x
      realtime). Parakeet TDT int8/CPU: ~3.6s model load (one-time ONNX
      session init — amortized away by a warm-context pool same as
      Whisper's), ~510–600ms inference (~20–23x realtime) — roughly 2x
      *slower* than our current default. This directly contradicts the
      earlier anecdotal "CPU beats Metal on M3" data point — didn't
      hold up here. Confirms the `parakeet-rs` source's own code comment
      (not just the README): CoreML is skipped entirely by design, not
      just "unstable" — the model's dynamic input shapes prevent CoreML
      from building an optimized ANE/GPU plan, so it silently runs on
      CPU anyway with extra overhead. CPU is the correct execution
      provider for this model on Apple Silicon, not a fallback.
    - **Accuracy: excellent, and differently shaped than Whisper's.**
      Both engines transcribed the test clip essentially perfectly.
      Parakeet TDT natively emits punctuation/capitalization without
      any prompt engineering; Whisper needs (and got, via
      `default_vocabulary()`'s initial-prompt trick) a nudge to reach
      the same polish — and that same prompt over-eagerly camelCased a
      plain-English phrase in the test clip, which Parakeet (no
      vocabulary-biasing hook at all) transcribed literally instead.
      Net: Parakeet's raw output quality looks strong, but it has no
      equivalent of our vocabulary list feature to bias toward
      dev-jargon — that hook would need to be built from scratch against
      `parakeet-rs`, unclear how (or if) it's even exposed.
    - **Footprint: substantially heavier.** The int8 TDT model alone is
      ~640MB on disk (622MB encoder + 17MB decoder_joint) — over 10x our
      current default (`base.en`, 59MB) and 3x larger than our biggest
      current option (`small.en`, 190MB). Bundling ONNX Runtime is a
      solved problem, not a blocker — `superwhisper.app` already ships
      `libonnxruntime.1.19.0.dylib` at 53MB in `Contents/Frameworks` —
      but it's real added weight on top of whisper.cpp's static lib.
  - **Recommendation, updated**: on real hardware, Parakeet is not a
    speed win over our current Metal-accelerated default — it's slower
    and much heavier on disk, for comparable accuracy on this (clean,
    single-voice, synthetic) test clip. That test can't probe the
    scenario where Parakeet might actually earn its footprint: harder
    real-world audio (background noise, accents, fast/mumbled speech).
    Before building any Settings/download UI, the next validate-first
    step is a harder audio test, not integration work — if accuracy
    doesn't meaningfully separate on hard audio too, the size+speed
    cost isn't worth carrying as a second inference backend.

- **Per-repo settings file (`.mouthpiece.json`)** (idea captured
  2026-10-20, from the product-strategy pass; user: backlog it, don't
  build yet). A file committed to a repo holding team vocabulary, spoken
  aliases and snippets, picked up automatically whenever Mouthpiece
  detects you're working in that repo (rides on codebase-aware
  vocabulary's project detection). Layering: automatic codebase names,
  then the repo file, then your personal settings win.
  - Why it matters beyond convenience: it's the one growth loop available
    to a local, free app. Every repo carrying the file advertises Dev
    Whisper to everyone who clones it.
  - Open questions: schema versioning, whether repo-provided snippets need
    a trust prompt before they can expand (a cloned repo shouldn't be able
    to inject text silently), and how to show which settings came from the
    repo.
  - Deliberately deferred: everything stays in-app for now.

- **"Look at this": screenshot attach** (idea captured 2026-10-20). Saying
  "look at this" grabs the frontmost window, or a dragged region, onto the
  clipboard alongside the dictated text so a coding agent (Claude Code,
  Cursor) gets the image with the prompt. Needs Screen Recording
  permission, so it should be opt-in and appear in the setup checklist
  only once enabled.

- **Structured prompts from long rambles** (idea captured 2026-10-20).
  One-shot, not the multi-turn Prompt-building mode below: dictations
  over ~80 words in a coding-agent context get a light structure (Goal,
  Context, Constraints, Done when), keeping every requirement and adding
  nothing. Toggle per mode. Needs a golden-case set that checks no
  requirement is dropped.

- **Pinned paste target** (idea captured 2026-10-20). Lock where dictation
  lands, then read docs or browse in another window while talking; the
  paste goes to the pinned window and focus returns. Shares the
  focus-hop machinery with the reply-to-waiting-agent feature.

- **"Sounds like" hints in the vocabulary** (idea captured 2026-10-20).
  A vocabulary entry can carry how you say it ("kube cuddle" ->
  `kubectl`, "post gress" -> `Postgres`), applied as a deterministic
  replacement after transcription. Overlaps with learned corrections;
  build on whatever replacement mechanism that ships.

- **Voice tune-up during setup** (idea captured 2026-10-20). Read 8 short
  jargon-heavy sentences; the app measures accuracy (WER) and speed for
  each downloaded model on your voice and mic and recommends one. Also
  feeds measured speeds into the model pickers.

- **"Proof it's local" panel** (idea captured 2026-10-20). A live count of
  outbound network bytes (normally zero, apart from update checks and
  model downloads, which are labelled), plus an optional offline lock that
  blocks everything except localhost Ollama. Turns the privacy claim into
  something users can check.

- **Rename the app: done 2026-10-26, now Mouthpiece** (was Dev Whisper).
  Why: a Mac App Store app called "DevWhisper" (devwhisper.app) does
  AI-refined dictation with a site close to ours, and "whisper" is crowded
  (Wispr Flow, Superwhisper, MacWhisper). Runner-up names were Sotto,
  Hushprompt and Prompt Aloud. On 2026-10-26 mouthpiece.dev and
  mouthpiece.app had no DNS records and no App Store app used the name;
  the domains aren't registered yet.
  - Bundle identifier `dev.mouthpiece` (was `com.devwhisper.app`).
    `rename.rs` moves the old data folder over (leaving a link at the old
    path), renames an updated `Dev Whisper.app` to `Mouthpiece.app`, and
    replaces the old login item. macOS permission grants can't move, so
    the setup checklist asks again.
  - `mouthpiece://` links, with `devwhisper://` still accepted; the
    "Dev Whisper, …" wake phrase still works.
  - Repo `JFinchy/mouthpiece` (GitHub forwards the old repo's URLs, so the
    updater and old download links keep working; releases also carry
    `Dev-Whisper.dmg`). The old site URL, jfinchy.github.io/dev-whisper,
    doesn't forward.
  - Left as is: the dev signing identity and identifier (renaming them
    resets the dev build's permission grants), the updater key file
    names in `~/.tauri/`, and the `tauri-app` binary name.

- **New app icon** (idea captured 2026-10-22). The current icon (code
  brackets over dense sound-wave bars) is busy at small sizes. A prompt
  for exploring directions with an image model is in
  `docs/icon-prompt.md`; best done together with, or after, a rename.

- **Meeting mode: separate speakers** (planned 2026-10-23, not started).
  Record a whole meeting and get a transcript split by speaker ("You:",
  "Speaker 2:", …), entirely on the Mac.
  - **Feasibility: good.** sherpa-onnx 1.13.5 (already a dependency) has
    `OfflineSpeakerDiarization`: pyannote segmentation, a speaker
    embedding, then clustering. The embedding model it needs
    (`wespeaker_en_voxceleb_resnet34_LM.onnx`) is already bundled for
    Isolated Voice; the only new file is the pyannote segmentation model
    (about 6 MB, downloaded on first use like the Whisper models, with a
    checksum).
  - **Pipeline.** A meeting recording is a separate capture from
    dictation (no paste, no hotkey hold): a "Start meeting" control in the
    widget and menu bar. After stopping: diarize the whole file → merge
    short turns → transcribe each turn with Whisper (base.en is fast
    enough: about 1 hour of audio in a few minutes on Apple Silicon) →
    label turns. If Isolated Voice is enrolled, the cluster closest to the
    enrolled voice is labelled "You". Output goes to History as one
    meeting entry (and a Markdown export), with an optional local Ollama
    summary.
  - **Phase 1: in-person (mic only), about 2-3 days.** Everyone in the
    room through one mic. Measure diarization error on a few real
    recordings with 2-4 speakers before going further; overlapping speech
    is where it will be weakest.
  - **Phase 2: calls, about 1 week more.** The other side of Zoom, Meet
    or Teams comes through the speakers, not the mic, so it needs system
    audio capture (ScreenCaptureKit audio, macOS 13+, Screen Recording
    permission), mixed with the mic or, better, kept as two channels (mic
    = you, system = everyone else) so "You" needs no guessing.
  - **Risks.** Long recordings need streaming to disk rather than one
    in-memory buffer; speaker count has to be estimated (or set by the
    user); consent: recording other people needs a clear on-screen
    indicator and a note in the UI that the user is responsible for
    telling participants.

## Research / Exploration

- **Competitive landscape: dictation/Whisper tools beyond SuperWhisper**
  (researched 2026-08-17, via live browsing — WebSearch/WebFetch were down
  platform-wide, real infra outage, not fabricated data) — covered Wispr
  Flow, MacWhisper, VoiceInk, and Talon Voice as a different-category
  reference point.
  - **Landscape**: two real pricing/architecture clusters exist, not one.
    *Cloud/hybrid subscription*: Wispr Flow ($0 free tier capped at
    2,000 words/week, $12/user/mo Pro, Enterprise; SOC2/HIPAA/ISO 27001;
    cloud-processed) and SuperWhisper (hybrid, see prior entry above).
    *Local one-time-purchase*: MacWhisper (free + €64 lifetime Pro,
    local Whisper/Parakeet models, optional bring-your-own-key cloud AI
    layer) and VoiceInk (100% offline, open source GPLv3, $29-69
    lifetime tiered by device count, no subscription at all). Dev
    Whisper is local *and* fully free — a combination none of the four
    match.
  - **VoiceInk is our closest architectural peer**: whisper.cpp-based,
    app-aware "Modes" that auto-apply settings per app/URL (same concept
    as our `modes.rs`), personal dictionary, global push-to-talk
    shortcuts, launch-at-login — all things we also have. Meaningful
    gaps where *they're* ahead: on-screen/selected-text context
    awareness is already shipped (via a library called SelectedTextKit,
    a real macOS API for reading text selections) — validates our own
    unbuilt "Now" item #3 (selected-text context feeding LLM
    refinement) is both achievable and worth prioritizing, and points at
    a concrete implementation path. They've also already shipped
    Parakeet model support (relevant data point for our
    validate-first-spike item, though doesn't resolve our own
    CoreML-instability concern since their acceleration path is
    unconfirmed) and a built-in conversational "AI Assistant" mode,
    a real step toward PRODUCT_SPEC.md §5's "Rubber Ducking" vision.
  - **Wispr Flow has a dedicated developer product page**
    ("Flow for Developers"): recognizes dev terms/camelCase/snake_case,
    tags files in Cursor/Windsurf to bring code context into AI prompts,
    and has a **snippet library** — spoken cue expands to a saved block
    of text (PR checklist, calendar link, onboarding instructions). This
    is a new pattern, distinct from a vocabulary list (which is about
    recognition accuracy, not insertion) — not currently tracked
    anywhere in our roadmap.
  - **MacWhisper has CLI control plus output-side workflow automation**:
    "Control MacWhisper from the CLI. Hook it up in your agent or
    scripting workflows," and auto-uploads transcripts to Notion,
    Zapier, Obsidian, n8n, Make.com, or a custom webhook. Our deep-link
    hooks (shipped 2026-08-16) cover the *input* side (start/stop
    recording via script); nothing covers routing a finished
    transcript/journal-summary *out* to another tool — a real gap this
    surfaced, not previously tracked.
  - **Talon Voice** is a different category worth naming explicitly, not
    a feature-parity competitor: full hands-free computer control (voice
    + eye tracking + noise clicks), Python-scriptable, free/donation —
    the actual incumbent tool serious RSI-affected/hands-free-coding
    developers already use. Not dictation-with-formatting like the rest
    of this list; not worth chasing feature parity with, but worth
    knowing it's what that specific audience segment compares us
    against. Follow-up deep dive (2026-08-17) on Talon specifically —
    what it actually offers, and which pieces are worth adapting — is in
    `COMMAND_MODE.md`.
  - **New, previously untracked findings to fold into ROADMAP.md**:
    output-side workflow automation (Notion/webhook/Slack routing for
    transcripts), and a snippet library (spoken-cue text expansion,
    separate from the vocabulary editor).

- **Ambient/hands-free listening mode** (idea captured 2026-08-23, direct
  user request — "go further with the listening mode"). Push-to-talk
  today is a deliberate bounding signal: exactly the audio between
  hotkey-down and hotkey-up gets transcribed and pasted. This would drop
  that requirement for an opt-in session where speech is auto-detected
  and transcribed continuously instead.
  - The hard part isn't detection, it's avoiding disruption — an open
    mic that auto-pastes every detected utterance would just as happily
    fire on a side conversation, a phone call, or the TV. Proposed v1
    scope: an explicit **session** (started/stopped via the existing
    double-tap-Fn gesture from the doubletap.rs work, not a true
    always-on background listener), gated behind Isolated Voice mode's
    enrolled-speaker check already being on — unenrolled ambient
    listening isn't worth shipping, the false-trigger rate would be too
    high.
  - `isolate.rs`'s hysteresis energy-gate (separate enter/exit
    thresholds, hangover padding) is most of the needed VAD primitive
    already, but it currently *masks* a single already-captured
    push-to-talk buffer. Ambient mode needs it repurposed to *segment* a
    live continuous stream into discrete utterances, each independently
    sent through transcription/formatting/paste — a real change in how
    it's used, not just a config flag.
  - Wake-word activation ("hey dev whisper") is a plausible later
    addition but not needed for v1, since the double-tap gesture already
    gives an explicit, low-friction start/stop signal.
  - Not yet scoped: whether ambient mode should respect per-app Modes
    (e.g. only auto-listen while a specific app is frontmost) or run
    independent of app context.

- **Prompt-building mode** (idea captured 2026-08-23, direct user
  request). A conversational assistant for turning a rough spoken idea
  into a refined LLM prompt: dictate a rough ask, the local LLM (existing
  Ollama pipeline in `llm.rs`) asks a clarifying question or names an
  assumption worth confirming, you answer by voice, repeat until the
  prompt is ready to hand off.
  - This is a genuinely different interaction shape than the rest of the
    app — multi-turn conversation state instead of one-shot
    dictate-transcribe-format-paste — so it's better modeled as a new
    top-level feature (peer to Snippets/Backtrack) than as a 4th
    `Behavior` value or another entry in the Modes list built out below.
  - **Text-to-speech, resolved**: yes, worth building, and cheaper than
    it sounds. macOS ships `AVSpeechSynthesizer` (and the `say` CLI as an
    even lighter integration point) — fully local, zero new model
    weight, no cloud call, a perfect fit for the app's local-only
    positioning. Speaking the clarifying questions back (toggleable, off
    by default probably, given every other synthetic-audio surface in
    this app is currently silent) lets the whole loop stay hands-off/
    eyes-off-screen instead of breaking immersion to read text.
  - Needs a session end condition (spoken "looks good"/"done", or a
    turn cap) and a decision on where the final prompt goes — clipboard,
    paste into frontmost app, or a dedicated review screen before
    delivery, given it's a synthesized multi-turn artifact rather than a
    direct transcript.
  - Not yet scoped: UI surface for showing the running draft + question
    while mid-conversation (widget flyout is probably too small; likely
    needs its own small window).

- **Dev-speak/business-speak coaching with active recall** (idea
  captured 2026-08-23, direct user request). Passively notice
  under-used target vocabulary/phrasing in what's actually dictated, and
  coach toward better word choices via spaced-repetition-style active
  recall drills.
  - Splits into two halves with very different cost. **Detection** is
    nearly free — `insights.rs` and the Vocabulary editor already have
    the per-dictation word-level data this would ride on, it's mostly a
    new comparison pass over data already flowing through the pipeline.
    **The active-recall quiz/flashcard UI** is a real new product
    surface — a spaced-repetition trainer bolted onto Settings, with its
    own review-scheduling logic, unrelated to the dictation loop itself.
  - Flagged as the idea here most likely to cut against the "invisible
    dictation tool" positioning (see the SuperWhisper research's
    explicitly-out-of-scope list) rather than extend it, since it turns
    part of the app into a standalone habit-building product. Worth a
    deliberate positioning call before building the recall-quiz half,
    not something to default into.

- **Mobile app keyboard suggestions** (idea captured 2026-08-23, direct
  user request). Unscoped between two different things: (a) a
  text-prediction/phrase-suggestion custom keyboard, vs (b) a
  dictation-enabled custom keyboard (voice-to-text via a mic button),
  mirroring the desktop app on iOS.
  - Meaningfully the largest-scope idea in this list — a second
    platform (iOS app + keyboard extension target, Apple Developer
    provisioning, TestFlight/App Store distribution), not a feature
    addition to the existing macOS app.
  - Apple's keyboard-extension sandbox changes the constraints
    significantly versus this app's current assumptions: no easy
    background mic capture, restricted/discouraged network access even
    with "Full Access" granted, tight memory limits. A from-scratch
    on-device Whisper pipeline on iOS is plausible (whisper.cpp has a
    CoreML iOS path, and prior art exists in other OSS dictation
    keyboards) but is a multi-week build, not a spike.
  - Recommendation: needs its own scoping research pass (what's the
    smallest real version worth shipping — e.g. syncing vocabulary/
    snippets to a companion keyboard vs. full on-device dictation) before
    it's estimated like a normal backlog feature.
