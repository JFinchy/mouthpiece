# LLMs from LM Studio / llama.cpp / Jan, and adding a .gguf to Ollama

Branch: `main` (uncommitted, 2026-11-05)
Files changed: `src-tauri/src/local_llm.rs` (new), `src-tauri/src/llm.rs`,
`src-tauri/src/readiness.rs`, `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`
(`tauri-plugin-dialog`), `src/settings/sections/llm.tsx`,
`src/settings/sections/modes.tsx`, `src/settings/shared.tsx`,
`src/testing/mockBackend.ts`, `e2e/settings.spec.ts`

## What to verify

For Macs where Ollama can't download models (enterprise proxy) but a model
is already on disk. Unit tests use fake servers and the e2e test uses the
mock backend; only a real LM Studio / Ollama shows the actual API shapes,
chat templates and the native file picker work.

## Steps

LM Studio (or Jan / `llama-server`):
1. In LM Studio, load a Gemma model, then Developer → Start server (port 1234).
2. Settings → LLM: the model is listed with an "LM Studio" tag. Click Use.
3. Settings → Modes: turn on "Refine with global LLM" for the mode of the
   app you'll dictate into (or pick the LM Studio entry for that mode).
4. Dictate "um so I think we should uh ship it tomorrow" into TextEdit:
   cleaned text pastes; `app.log` shows a `prewarmed ... on LM Studio` line
   and no errors. Settings → LLM shows a ~latency next to the model.
5. Stop LM Studio's server. Settings → Setup checklist shows
   "LM Studio isn't running"; dictation still pastes (unrefined).
6. With `llama-server -m model.gguf --port 8080` instead: listed as
   "llama.cpp server", refinement works.

Model file → Ollama (Ollama running, nothing pulled needed):
7. Settings → LLM → "Model files on this Mac" lists `.gguf` files from
   `~/.lmstudio/models` (when LM Studio's server is off), the Hugging Face
   cache and `~/models`, with size and location.
8. Click "Add to Ollama": "Checking the file N%", "Copying into Ollama N%",
   "Setting up the model…", then "In Ollama ✓". The model appears in the
   list above (named after the file, e.g. `gemma-3-4b-it-q4_k_m`). `ollama
   list` shows it too.
9. Use it and dictate as in step 4: cleaned text, no odd tokens like
   `<end_of_turn>` (proves the chat template came along).
10. "Choose a .gguf file…" → pick one from Downloads: no macOS "access
    Downloads" prompt; it's added the same way.
11. Pick a non-GGUF file renamed to `.gguf`: "doesn't look like a GGUF model".
12. Quit Ollama, click "Add to Ollama" on another file: "Ollama isn't
    running. Start Ollama and try again…".

## Pass criteria

Local-server models are listed, refine dictation and report readiness
correctly; a `.gguf` becomes a working Ollama model without any download;
failures show plain-language errors.

## Automated checks done (2026-11-06)

Against a real Ollama 0.40.13 with the `gemma3:1b` .gguf already on disk,
the import's API calls (`HEAD /api/blobs/sha256:<digest>` → 200,
`POST /api/create` with the `files` map → success) created a model that
generated text, then was deleted. The blob already existed, so the upload
step (`POST /api/blobs`) was covered only by the fake-server tests. LM Studio,
llama.cpp and Jan weren't installed on this machine.
