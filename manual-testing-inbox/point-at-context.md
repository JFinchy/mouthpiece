# Point at things: selection, diff, screenshot, file:line

Branch: `agent-fleet`
Files changed: src-tauri/src/attachments.rs, error_context.rs, names.rs, pipeline.rs, recording.rs, project.rs

## What to verify

Spoken attach commands read real app state (selection via Accessibility, git, screen capture), which the tests only cover as pure logic.

## Steps

1. In VS Code, select a few lines and, with VS Code still in front, dictate "Explain this, attach the selection." The selection is appended in a code fence.
2. In a repo with uncommitted changes, in a terminal, dictate "Review this, attach the diff." A ```diff block of `git diff HEAD` follows. Stage one file and say "write the message, attach the staged changes": only the staged file.
3. In a clean repo, "attach the diff": your words paste and the widget says "No changes to attach".
4. With Safari behind your terminal, in the terminal say "Why is the header red, look at this." The first time, macOS asks for Screen Recording; allow it (may need a relaunch) and repeat. The paste ends with `@/var/folders/…/mouthpiece-screenshots/screenshot-….png`, and that PNG shows the Safari window, not the terminal. Claude Code can open it.
5. Say "I'll look at this later." It pastes as spoken (no screenshot).
6. In a terminal in this repo, say "check line 42 of the paste file". It pastes `check @src-tauri/src/paste.rs:42`.

## Pass criteria

Each attachment holds the right content, nothing is attached for ordinary sentences, and failures leave your words intact with a note.
