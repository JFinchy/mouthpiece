# Testing

Four layers, fastest first. CI (`.github/workflows/ci.yml`) runs all of
them on every push to `main` and every pull request.

| Command | What it covers | Time |
|---|---|---|
| `bun run test:rust` | ~250 Rust unit tests, including the golden dictation suite | ~10s |
| `bun run test:e2e` | Playwright UI tests in WebKit against a fake backend, plus a pass under the production CSP | ~25s |
| `bun run test:audio` | Real speech → Whisper → pipeline, with a word-error-rate report | ~30s |
| `bun run test` | typecheck + Rust + UI e2e | ~40s |

Plus manual verification of anything hardware-dependent, tracked in
[`manual-testing-inbox/`](manual-testing-inbox/README.md).

## Golden dictation cases

`src-tauri/tests/fixtures/pipeline_cases.json` lists spoken input and the
exact text that must be delivered, run through the real pipeline
(`src-tauri/src/pipeline.rs`) with a scripted LLM:

```json
{ "name": "backtrack swaps the corrected word",
  "input": "Let's meet at two, actually three.", "mode": "plain",
  "expect": "Let's meet at three." }
```

Optional fields: `llm` (`{"returns": "…"}` or `{"error": "…"}`, which turns
LLM refinement on for the case), `press_enter_enabled`, `expect_prefix`,
`expect_label`, `expect_press_enter`, `expect_append_clipboard`,
`expect_llm_called`.

**Every dictation bug gets a case.** When a paste comes out wrong, turn
on Settings → History → Privacy → "Include transcript text in logs"
(off by default, so logs show only `<N chars>`), reproduce it, and grab
the `whisper: raw transcript=` line from `app.log`. Add it as `input`,
with what should have been pasted as `expect`, and fix the code until
`cargo test --lib pipeline` passes.

## Real-audio e2e

`src-tauri/src/e2e_audio.rs` speaks each phrase in
`src-tauri/tests/fixtures/audio_cases.json` with macOS `say` (Samantha,
16kHz), transcribes it with the real Whisper model, formats it with the
real pipeline, and compares against `expect`. It reports word error rate
(case and punctuation ignored) and transcription time:

```
case                            WER  stt ms  output
snake case                    0.000     266  "error_response_handler"
spoken list                   0.000     269  "1. Fix the tests.\n2. Ship it."
model ggml-base.en-q5_1.bin: mean WER 0.033, mean stt 315ms over 10 cases
```

This layer catches what unit tests can't: Whisper writing a spoken
casing command as `snake_case_…`, turning "One, … Two, …" into digits, or
hearing "comma" as "Kama" all broke real dictation while every unit test
passed.

Two more tests run meeting mode on a scripted conversation between macOS
voices: speaker separation has to give each voice one speaker of its own
(on "work it out" with two voices, and with the count given for three,
two of them close), and the transcript stays under 15% WER.

`max_wer` sets a tolerance for a case, and `note` says why. To compare
models, point it at another file:
`DW_E2E_MODEL=/path/to/ggml-small.en-q5_1.bin bun run test:audio`.

`live_drafts_show_words_before_the_phrase_ends` streams a long sentence
with no pause and checks that draft text (the quick pass,
`WhisperEngine::transcribe_preview`) shows up seconds before the phrase is
cut into a chunk; `quick_pass_is_faster_on_short_clips` times the quick
and full passes against each other.

The audio tests run one at a time (`--test-threads=1`): Whisper's progress
counter is process-wide, so two transcriptions running at once would mix
up each other's progress checks.

## UI e2e

`e2e/` drives the real React app in WebKit (Tauri's engine on macOS),
with `src/testing/mockBackend.ts` standing in for Rust:

- `open(page, "widget", scenario)` starts a window with a scenario: setup
  problems, devices, a mic in fallback, commands that should fail.
- `emit(page, "recording-started")` plays a backend event.
- `calls(page, "set_input_device")` returns what the UI sent.

Every test fails on an uncaught exception or unexpected console error.
Tests tagged `@csp` also run against a production build served with the
CSP from `tauri.conf.json`, so a policy violation fails CI rather than
shipping a blank window.

`bun run dev:mock` serves the same fake backend for UI work in a browser.

## LLM evals

`cargo test --lib eval_suite_against_real_ollama -- --nocapture` runs the
refinement prompts against your local Ollama (skipped when it isn't
running). Override the model with `DW_EVAL_MODEL=gemma3:4b`.
