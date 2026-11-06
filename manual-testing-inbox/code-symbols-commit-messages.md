# Spoken code symbols and commit messages

Branch: `agent-fleet`
Files changed: src-tauri/src/punctuation.rs, commit_message.rs, llm.rs, pipeline.rs

## What to verify

Symbol phrases change what pastes in every app; commit messages need a real model and staged changes.

## Steps

1. Dictate "const add equals open paren a comma b close paren fat arrow a plus b": `const add=(a, b) => a+b`.
2. Dictate "if x triple equals y double ampersand ready" and "std double colon fs": `if x === y && ready`, `std::fs`.
3. Dictate ordinary sentences with "arrow", "pipe", "dot": unchanged.
4. Stage a change, then in the terminal (with `git commit` open in your editor, or anywhere) say "Commit message: fixed the race in paste." A proper commit message (imperative summary line) pastes.
5. With nothing staged, the same pastes your words as spoken.

## Pass criteria

Symbols come out right without touching prose; commit messages describe the staged change.
