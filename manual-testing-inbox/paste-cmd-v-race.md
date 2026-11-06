# Paste never arrives as a bare "v"

Branch: `main`
Files changed: `src-tauri/src/paste.rs`

## What to verify

Every so often a dictation typed a lone "v" instead of the transcript, while
the transcript sat on the clipboard. Cause: the paste sent Cmd and V as
separate synthetic keys, and the V only counted as Cmd+V if macOS had already
absorbed the Cmd keydown (a 40ms wait that a busy window server can outlast).
Cmd+V and Cmd+Z ("scratch that") are now sent with the Command flag on the
letter's own events. Measured with the stress test below: old 3/60 bare v
under CPU load at the shipped timing, 50/60 with no wait; new 0 in both.
A human still needs to confirm pasting in the apps actually used day to day.

## Steps

1. Build from `main` (`bun run tauri build` or the next release) and install.
2. Dictate ~20 short phrases each into: a terminal (Terminal/iTerm/Warp), an
   Electron app (VS Code, Slack), a browser text field, and Notes/TextEdit.
3. Do a few right after a long dictation (60s+) and with LLM refinement on,
   when the machine is busiest.
4. Hold Option (or Shift) while a paste lands: the text should paste plainly,
   not "paste and match style" or anything else.
5. Say "scratch that" right after a paste in Notes: the paste is undone.
6. After all of it, type normally: no modifier (Cmd) feels stuck.
7. Optional, with the terminal granted Accessibility and hands off the
   keyboard for ~40s (it drives TextEdit and stops if focus leaves it):
   `cd src-tauri && PROBE_LOAD=24 cargo test --lib paste::tests::stress -- --ignored --nocapture`

## Pass criteria

No bare "v" (or "z") in any app across the runs, every transcript pastes
where the cursor was, no stuck Cmd afterwards, and step 7 prints
`new: 60/60 pasted, 0 bare v`.
