# Agent prompt voice commands: send it, attach the error, scratch that

Branch: `worktree-agent-a8f87a24f94035051`
Files changed: `src-tauri/src/error_context.rs` (new), `src-tauri/src/undo.rs` (new),
`punctuation.rs`, `pipeline.rs`, `recording.rs`, `paste.rs`, `app_detect.rs`,
`src/settings/sections/dictation.tsx`

## What to verify

Three trailing/standalone voice commands for prompting coding agents. The
pure logic (phrase rules, error-block extraction from real-looking tool
output, undo planning) is unit-tested, but nothing automated exercises the
macOS Accessibility read of a real terminal, real Cmd+Z / backspace
keystrokes, or the widget notes.

## Steps

1. `bun run tauri dev`. In Settings > Dictation turn on "Press enter".
2. **Send it.** Focus a terminal running a coding agent (or a shell).
   Dictate: "echo hello, send it" (pause after "hello"). Expect `echo hello`
   pasted and Enter pressed. Then dictate "I will send it tomorrow": it must
   be pasted as text with no Enter. Then "fix the bug and send it": text,
   no Enter.
3. **Attach the error (Terminal.app and iTerm2).** In each, run
   `cargo build` in a directory with a type error (or `python3 -c "raise ValueError('boom')"`).
   Leave the output on screen, then dictate "why does this fail, attach the error".
   Expect your sentence plus a blank line plus a fenced block holding the error
   output (not the earlier prompt lines), at most 60 lines.
4. **Selection wins.** Select part of the terminal output with the mouse and
   repeat step 3: the block is the selection.
5. **Clipboard fallback.** In a browser or editor, copy a stack trace, focus
   a text field, dictate "what is this, attach the error". Expect the
   trace in a fenced block. Repeat with a URL on the clipboard: expect only
   your words and the widget note "No error found to attach".
6. **Unreadable terminal.** In Warp or the VS Code terminal repeat step 3
   with nothing error-like on the clipboard: expect the note above, no crash.
7. **Scratch that, editor.** In Notes or TextEdit dictate "hello world",
   then say "scratch that" within 30s. Expect the text removed and the
   widget note "Undid the last paste". Say "scratch that" again: "Nothing to undo".
8. **Scratch that, terminal.** In Terminal.app type `ls ` (leave it on the
   line), dictate "-la src", then within 15s say "undo that". Expect only
   `-la src` removed; `ls ` stays.
9. **Refusals.** After a dictation, switch to another app and say "scratch
   that": "Couldn't undo safely here" and nothing changes. Same after waiting
   over 15s in a terminal, after a two-line paste in a terminal, and after
   "..., press enter".
10. **Backtrack unchanged.** Dictate "call at three scratch that four":
    pasted as "call at four".
11. Settings > Dictation > Voice Commands lists Send it, Attach the error,
    and Scratch that.
12. Check Settings > Logs: "attach error" lines show line counts only, no
    captured text.

## Pass criteria

Every expectation above holds, no text beyond what Mouthpiece pasted is
ever deleted, and the log never contains terminal or clipboard content.
