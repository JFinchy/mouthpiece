# Mouthpiece

_Formerly Dev Whisper._ Local, private dictation for developers on Apple Silicon. Hold a hotkey,
talk, and the text lands at your cursor, formatted for the app you're in:
prose in Slack, commands and `@file` tags in a terminal coding agent,
`snake_case` when you ask for it. Speech recognition (whisper.cpp on
Metal) and optional cleanup (Ollama) both run on your Mac; nothing is sent
anywhere.

Website: **[jfinchy.github.io/mouthpiece](https://jfinchy.github.io/mouthpiece/)**

## What it does

**Dictation that fits where you're typing**
- Press your hotkey (⇧⌘Space by default, or double-tap Fn), talk,
  press it again. The text pastes at your
  cursor, or just copies if you prefer.
- Knows which app you're in: sentences in Slack and email; `@file` tags
  and line breaks in Claude Code, Cursor and your terminal.
- Live progress while it works: amber while transcribing, violet while
  the LLM tidies up. Click the violet bar, or press the hotkey with
  Shift added (Option if it already uses Shift), to paste without the
  LLM step.

**Speaks developer**
- A vocabulary list, preloaded with dev terms you can edit, so `kubectl`,
  `JSON` and `Hammerspoon` come out right. Your list is also passed to the
  LLM so it doesn't "fix" them.
- Say "snake case error response handler" → `error_response_handler`.
  Camel, Pascal, kebab and screaming case work too.
- "Generate boilerplate for a React component named User Profile" writes
  the code.
- Backtrack: "let's meet at two, actually three" → "let's meet at
  three."
- Spoken lists, punctuation ("period", "new line"), "press enter" to
  submit (turn it on in Settings), and "append clipboard" to paste a log or URL after your words.
- Snippets: a short spoken cue expands into a saved block of text.

**Local and private**
- Speech recognition runs on your Mac (whisper.cpp on Metal). LLM cleanup
  is optional and runs in Ollama, also on your Mac.
- Nothing about your speech or text is sent anywhere. The only network
  traffic is update checks and model downloads.
- The LLM only runs when the transcript needs it, so short, clean
  dictations skip it.

**Hands-free, if you want it**
- Always-on listening: say "Mouthpiece, …" and only your enrolled voice
  triggers it. It pauses automatically during calls.
- Isolated Voice filters out other people talking near you.

**Built for automation**
- Start and stop recording from scripts or other tools with
  `open mouthpiece://start-recording` (and `stop-` and `toggle-`).
- A webhook can receive each finished transcript.
- Searchable History lets you re-run any past dictation through a
  different mode.
- Insights show your speaking speed, streaks and per-app usage.

The full, detailed list is in [`FEATURES.md`](FEATURES.md).

## More docs

- Everything that's built: [`FEATURES.md`](FEATURES.md)
- Where it's going: [`ROADMAP.md`](ROADMAP.md), [`BACKLOG.md`](BACKLOG.md),
  [`PRODUCT_SPEC.md`](PRODUCT_SPEC.md)
- How it's tested: [`TESTING.md`](TESTING.md)
- Cutting a release: [`RELEASING.md`](RELEASING.md)

## Install

**[Download Mouthpiece for Mac (Apple Silicon)](https://github.com/JFinchy/mouthpiece/releases/latest/download/Mouthpiece.dmg)**:
open it and drag Mouthpiece into Applications. Always the newest
version; older ones are on the
[Releases](https://github.com/JFinchy/mouthpiece/releases) page. The app isn't notarized yet, so macOS blocks
the first launch: open it once, then go to **System Settings → Privacy &
Security**, scroll down and click **Open Anyway** (on macOS 14 and
earlier, right-click the app → **Open** instead).

Two ways to skip that step:

- **One-line install.** Downloads the same DMG from GitHub, checks the
  app's signature and copies it into Applications. Files downloaded with
  `curl` aren't flagged, so macOS doesn't prompt. The script is
  [`site/install.sh`](site/install.sh).
  ```sh
  curl -fsSL https://jfinchy.github.io/mouthpiece/install.sh | sh
  ```
- **Build from source.** Needs [Rust](https://rustup.rs),
  [Bun](https://bun.sh) and the Xcode command line tools
  (`xcode-select --install`):
  ```sh
  git clone https://github.com/JFinchy/mouthpiece && cd mouthpiece
  bun install
  bun run tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
  ditto "src-tauri/target/release/bundle/macos/Mouthpiece.app" "/Applications/Mouthpiece.app"
  ```
  (The config flag skips the signed update package, which needs the
  project's private key.)

On first launch, Settings shows a setup checklist until everything's in
place:

1. **Speech model**: download one under Voice (Base, ~57MB, is a good
   default).
2. **Pasting**: allow Mouthpiece under System Settings → Privacy &
   Security → Accessibility (the checklist's button opens that page).
   Not needed if you turn on copy-only delivery.
3. **Microphone**: macOS asks on the first recording.
4. **Optional, LLM cleanup**: install [Ollama](https://ollama.com) and
   download a small model from Settings → LLM.

Updates install themselves: the app checks GitHub on launch, shows a dot
on the widget's gear when a new version is out, and Settings → About
installs it. Update packages are signature-checked.

## Development

```sh
bun install
./scripts/download-model.sh   # Whisper base.en into src-tauri/models (checksum-verified)
bun run tauri dev             # the app, with hot reload
bun run dev:mock              # just the UI in a browser, on a fake backend (http://localhost:1430/?window=settings)
bun run test                  # typecheck + Rust tests + UI e2e
```

`dev:mock` runs the React UI against the in-memory backend in
`src/testing/mockBackend.ts`, so UI work doesn't need a Rust rebuild.
Append `?window=widget` or `?window=settings` to pick the window.

### Stable dev signing

macOS ties Accessibility and Input Monitoring grants to an app's code
signature, and every `cargo` build is signed with a new ad-hoc one, so
each rebuild used to lose those grants. `.cargo/config.toml` runs
`scripts/dev-sign-runner.sh`, which re-signs the dev binary with a fixed
local identity before launching it. One-time setup, in your own Terminal
(the trust step asks for your Mac password):

```sh
./scripts/setup-dev-signing.sh
```

Then run `bun run tauri dev`, click **Always Allow** if `codesign` asks to
use the key, and grant Accessibility to the dev app one last time.

Without the certificate the runner launches unsigned and says so. If
signing stalls on a keychain prompt, it gives up after 20 seconds and
launches unsigned rather than hanging.

### Project layout

| Path | What's there |
|---|---|
| `src-tauri/src/recording.rs` | The dictation flow: record → transcribe → format → deliver |
| `src-tauri/src/pipeline.rs` | Text formatting, pure and testable (lists, punctuation, backtrack, commands, modes, LLM) |
| `src-tauri/src/readiness.rs` | Setup checklist |
| `src-tauri/src/{audio,stt,llm,paste}.rs` | Mic capture, Whisper, Ollama, paste |
| `src-tauri/src/ambient.rs` | Hands-free "Mouthpiece, …" listening |
| `src/WidgetView.tsx` | The floating widget |
| `src/SettingsView.tsx`, `src/settings/` | Settings window, one file per page |
| `e2e/` | Playwright UI tests |
| `src-tauri/tests/fixtures/` | Golden dictation cases and real-audio cases |

Logs: `~/Library/Application Support/dev.mouthpiece/app.log` (also
Settings → Advanced → Logs).

## Known limitations

- **iPhone (Continuity) microphone** is unreliable: the stream sometimes
  opens but delivers no audio. The widget now flags this as "No sound from
  mic" within ~2 seconds; use a built-in, USB or Bluetooth mic for
  anything important.
- **Not notarized**, so Gatekeeper needs the **Open Anyway** step (System
  Settings → Privacy & Security) on first install. Releases before 0.3.1
  weren't signed as a whole bundle, which macOS reports as "damaged"; for
  those, run `xattr -dr com.apple.quarantine "/Applications/Mouthpiece.app"`
  or install 0.3.1 or later.

## License

[MIT](LICENSE)
