# Command Mode (Talon-inspired)

Status: **proposed, not started, no code written**. This is a big initiative —
likely bigger than everything else currently in `ROADMAP.md` combined — so
it gets its own document instead of a `BACKLOG.md` bullet. See
`ROADMAP.md`'s Later section for where this sits relative to other work.

## Why

Talon Voice (researched 2026-08-17 — see [What Talon actually offers](#what-talon-actually-offers)
below) does something Mouthpiece doesn't: continuous, always-listening
voice **commands** mapped to actions, as opposed to push-to-talk
**dictation** that produces formatted text. The user's read after seeing
this: there's no reason Mouthpiece couldn't support both, as two distinct
modes rather than picking one. That's the premise of everything below —
not "replace push-to-talk dictation," but "add a second, opt-in mode next
to it."

This also isn't a request to *become* Talon. Talon is a mature, ~5-year-old
project with dedicated hardware integrations (eye tracking) and its own
speech engine. The scope here is deliberately narrower: take the two or
three pieces of Talon's model that genuinely extend Mouthpiece's existing
architecture, and be explicit about the pieces that don't fit and shouldn't
be built.

## What Talon actually offers

Everything below is from direct review of Talon's own sites/docs/repos and
one detailed third-party technical review, not secondhand summary. Cited
inline.

- **Command grammar, not freeform dictation.** Talon's default mode maps
  short spoken phrases to actions — `save file: key(ctrl-s)` is a real
  example line from a `.talon` grammar file. It does have a dictation mode
  too, but discrete commands are the default and the point.
  [Talon: In-Depth Review](https://handsfreecoding.org/2021/12/12/talon-in-depth-review/)
- **Always listening by default.** "Out of the box, Talon is always
  listening. The phrase 'go to sleep' will disable it, and 'wake up' will
  activate it again." Processing is local — the mic being hot is a
  UX/architecture necessity for hands-free control, not a cloud round-trip.
  [Four weeks of voice computing — Fileside](https://www.fileside.app/blog/2025-04-14_voice-coding)
- **Its own local speech engine** ("Conformer," built on Meta's
  wav2letter), separate from Whisper entirely, plus optional integration
  with Dragon NaturallySpeaking via a custom bridge ("Draconity") for users
  who want Dragon's language model instead.
  [Talon: In-Depth Review](https://handsfreecoding.org/2021/12/12/talon-in-depth-review/)
- **Extensible via Python.** Any voice phrase, keyboard shortcut, or even
  a *trained custom noise* (pop, hiss, whistle — via a community project
  called `parrot.py`) can be bound to arbitrary Python code as an "action."
  Actions defined in Python are callable from any `.talon` grammar file
  without imports. [Talon: In-Depth Review](https://handsfreecoding.org/2021/12/12/talon-in-depth-review/)
- **`knausj_talon` / `talonhub/community`** — the de facto community
  grammar almost everyone installs on top of the core platform. Ships
  ~80% of a power user's command vocabulary out of the box (window
  management, editing, navigation), plus a voice-searchable GUI help
  system. [talonhub/community on GitHub](https://github.com/talonhub/community)
- **Cursorless** — the standout feature for code editing specifically. A
  VS Code extension + Talon grammar that decorates on-screen code with
  colored "hats" over tokens, so a command like "cut air" operates on a
  specific token by name/color instead of by cursor position. Genuinely
  faster than mouse+keyboard for a fluent user, per multiple independent
  reviews. [cursorless.org](https://www.cursorless.org) ·
  [cursorless-dev/cursorless](https://github.com/cursorless-dev/cursorless) ·
  [cursorless-dev/cursorless-talon](https://github.com/cursorless-dev/cursorless-talon)
- **Talon HUD** — a community-maintained visual overlay showing current
  mode (awake/asleep, command/dictation) and recently-recognized commands,
  movable anywhere onscreen. Doubles as a tutorial layer for learning
  commands. [chaosparrot/talon_hud](https://github.com/chaosparrot/talon_hud)
- **Eye tracking** (Tobii hardware) as a full mouse replacement — click by
  looking + a trained noise (a pop sound), extends to Mac/Linux via a
  lightweight custom integration bypassing Tobii's own heavyweight
  software. [Talon: In-Depth Review](https://handsfreecoding.org/2021/12/12/talon-in-depth-review/)
- **Free to install, Patreon-supported** for early access/priority
  support — no hard paywall on the base app. Core Python API is partially
  closed-source (single primary maintainer, Ryan Hileman); community
  grammars (`talonhub/community`, Cursorless) are fully open source under
  their own licenses. [talonvoice.com](https://talonvoice.com) ·
  [Talon: In-Depth Review](https://handsfreecoding.org/2021/12/12/talon-in-depth-review/)

## Feature breakdown

Six pieces, scoped independently. A-C are the recommended core (build
these); D-F are named for completeness because the user asked for a
thorough accounting of what Talon offers, but are explicitly *not*
recommended to build ourselves, with reasoning for each.

---

### A. Command Mode — the core capability

A second, opt-in operating mode alongside today's push-to-talk dictation:
continuous listening, matching short utterances against a user-defined
phrase → action grammar, executing the bound action instead of
pasting formatted text. Not simultaneous with Dictation Mode — like
Talon, the two need an explicit switch, since a given utterance can't be
both "a command to execute" and "text to format and paste" at once.

**Tasks:**
- [ ] Design spike: continuous-listening audio pipeline. Today's
  `audio.rs` is explicit start/stop (hotkey press → record → hotkey
  release → stop). Command Mode needs voice activity detection (VAD) to
  segment continuous audio into discrete utterances with no explicit
  boundary — a materially different pipeline, not an extension of the
  existing one. This needs its own research pass before implementation
  work starts (candidate approaches: energy-based VAD, `webrtc-vad`-style
  crates, or a lightweight always-on Whisper pass with silence trimming).
- [ ] Define the command data model: `Command { phrase: String, action:
  CommandAction }`, `CommandAction` as an enum — start with
  `Keystroke(Vec<Key>)`, reusing `paste.rs`'s existing `rdev`
  keystroke-simulation infrastructure (currently hardcoded to Cmd+V; needs
  generalizing to arbitrary key combos).
- [ ] New `command_mode.rs`: owns the continuous-capture lifecycle,
  utterance segmentation, phrase matching (exact match against configured
  phrases to start — fuzzy/prefix matching is a v2 concern), and action
  dispatch.
- [ ] New `AppConfig` fields: `command_mode_enabled: bool`, `commands:
  Vec<Command>`.
- [ ] Settings UI: a new "Commands" section — add/edit/delete phrase →
  action pairs, following the existing Vocabulary/Mode Rules editor
  pattern already in `src/settings/sections/modes.tsx`.
- [ ] Mode switch: a hotkey or tray menu item to toggle between Dictation
  Mode (today's push-to-talk) and Command Mode (continuous). Mutually
  exclusive, not layered.
- [ ] Widget/tray indicator: Command Mode active needs to be as
  unmissable as the current recording red-dot, ideally more so — this is
  a genuinely different privacy posture (mic open continuously, not
  gated behind an explicit press) and needs to read as obviously
  different from idle at a glance, every time, not just on hover.

---

### B. Sleep / wake voice control

Once Command Mode exists: the "go to sleep" / "wake up" pattern from
Talon (see citation above), so continuous listening can be muted/unmuted
without touching the keyboard — the whole point of a hands-free mode.

**Tasks:**
- [ ] Reserve two built-in trigger phrases ("go to sleep", "wake up"),
  checked before general command matching.
- [ ] Distinct tray icon / widget state for asleep vs. awake vs. idle
  (three states now, not two) — this matters more here than it did for
  the existing recording indicator, since "asleep" and "idle/off" look
  identical unless clearly distinguished, and confusing the two is the
  exact failure mode noted in Talon's own user feedback (forgetting
  which state you're in).
- [ ] Setting to customize the sleep/wake phrases, so they don't collide
  with a user's own configured commands.

---

### C. Extensible command actions

Talon's real depth comes from arbitrary Python per command. Mouthpiece
doesn't need a general scripting runtime to get most of that value — it
needs a couple of escape hatches that let a command phrase drive whatever
automation the user already has (shell scripts, Raycast, Hammerspoon),
rather than us reinventing a plugin system.

**Tasks:**
- [ ] `CommandAction::RunShellCommand(String)` — execute a user-defined
  shell command when a phrase matches. Needs an explicit
  acknowledge-the-risk step in the Settings UI when a user adds one of
  these (arbitrary shell execution triggered by voice is meaningfully
  more dangerous than a keystroke binding, and should not be a silent,
  easy default).
- [ ] `CommandAction::OpenUrl(String)` — reuse the deep-link scheme
  already shipped (`mouthpiece://…`) so a command phrase can trigger any
  of Mouthpiece's own automation hooks, or hand off to any other app's
  URL scheme.
- [ ] Document the recommended pattern (bind a phrase to a Raycast
  script/Hammerspoon function/shell one-liner) so Mouthpiece becomes a
  voice front-end to a user's *existing* automation, instead of
  competing with it.

---

### D. Cursorless-style structural code editing — **not recommended to build**

Talon's most differentiated feature, and the one most worth naming
honestly as out of scope. Cursorless is a full VS Code extension (its own
TypeScript codebase) plus a much richer spoken grammar than
phrase-to-action mapping — addressing syntax tokens by decorated "hat"
color, not by saved phrases. This is a materially different engineering
effort from A-C above, in a different language/runtime, tied to one
specific editor.

**Recommendation:** don't build our own version. Document how to run
Talon + Cursorless *alongside* Mouthpiece for users who want structural
voice editing — Mouthpiece handles freeform dictation and Command Mode
for system-wide actions, Talon+Cursorless handles in-editor structural
edits. Revisit only if there's real, specific user demand for it — this
is not a "maybe later" backlog item, it's a "probably never, and that's
fine" one.

---

### E. Noise recognition & eye tracking — **not recommended to build**

Talon supports custom-trained noises (pop/hiss/whistle) as a third input
modality, and Tobii eye-tracking hardware as a full mouse replacement.
Both extend well past "dictation app" into full hands-free *computer*
control — a different product category and a different (primarily
accessibility-driven) audience than Mouthpiece's developer-dictation
positioning.

**Recommendation:** don't build. Named here only because the user asked
for a thorough accounting of what Talon offers. If a user needs full
hands-free control (not just dictation), the honest answer is "use
Talon" — not "wait for us to rebuild a worse version of it."

---

### F. Visual mode/command feed

Talon HUD's persistent overlay (current mode + recently-recognized
commands) is a small, genuinely useful piece that fits Mouthpiece's
existing UI investment.

**Tasks:**
- [ ] Extend the already-shipped Detailed widget mode to show the last
  few matched commands (phrase + which action fired) while Command Mode
  is active — same "persistent, non-truncating status area" the Detailed
  mode was already built for.
- [ ] Reuse the already-shipped in-app log viewer as the "why didn't my
  command fire" debugging tool — mirrors Talon's own `sim("utterance")`
  REPL debugging approach, without needing a separate debug surface.

## Sequencing

A → B → C, in that order — B and C both assume A exists. D and E are
explicitly not queued. F can land alongside or shortly after A, since it
reuses UI that's already built.

Realistic sizing: A alone (specifically the continuous-listening/VAD
pipeline) is a bigger lift than everything shipped in the 2026-08-15/16/17
sessions combined — it's a new audio architecture, not an extension of the
existing push-to-talk one. Worth a dedicated design/research session
before any implementation work starts, same recommendation already on
record for voice isolation and the Parakeet spike.
