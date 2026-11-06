# Structure long prompts (Goal / Context / Constraints / Done when)

Branch: `agent-fleet`
Files changed: src-tauri/src/structure.rs, llm.rs, pipeline.rs, modes.rs, settings/sections/modes.tsx

## What to verify

Depends on a real local model's answers; the golden cases only test the guard with scripted answers.

## Steps

1. Settings > Modes > CLI (or your terminal mode): tick "Structure long prompts". Have Ollama running with your usual model.
2. In a terminal, dictate a 30-second request with several requirements, a file name and a number. The widget shows the violet refining bar, then a Goal / Context / Constraints / Done when prompt pastes. Check every requirement, name and number you said is in it and nothing was invented.
3. Dictate a short request: it pastes as spoken, no LLM call.
4. Stop Ollama and dictate a long one: it pastes as spoken (app.log: `structure: … failed`).
5. If a structured version is ever rejected, app.log says `structure: rejected … (dropped N of M words)` and your words paste as spoken.

## Pass criteria

Long prompts come out structured with nothing lost; any doubtful answer falls back to your words.
