# Pauses don't leave "..." or "--"; "Open at login" setting

Branch: `main`
Files changed: `src-tauri/src/pauses.rs` (new), `src-tauri/src/pipeline.rs`,
`src-tauri/src/lib.rs`, `src-tauri/tests/fixtures/pipeline_cases.json`,
`src/settings/sections/dictation.tsx`, `FEATURES.md`

## What to verify

Whisper writes a pause to think as "..." or "--" ("for the... Amazon Prime",
"when the-- computer turns on"). Those are now removed, with the words joined
or, after a finished thought, a new sentence started. Unit and pipeline tests
use real examples from History, but whether the result reads right with your
own pauses needs a person. Also: the login toggle is now called "Open at
login" (what macOS calls it), with a one-line explanation.

## Steps

1. Dictate with deliberate mid-sentence pauses, e.g. "Can we add a setting
   for … [pause] … boot at startup when the … [pause] … computer turns on".
2. Dictate a finished thought, pause, then start a new one: "I was looking
   for a course … [pause] … Do we have one?"
3. Trail off at the end: "I was thinking … [pause]" then stop.
4. Say "wait for it ellipsis" and "this em dash that": the symbols still
   appear.
5. Dictate "git commit dash dash amend" into a terminal or say a hyphenated
   word ("well-known"): dashes are kept.
6. Spoken list: "Two things. One, fix the tests. Two, ship it." still
   becomes a numbered list.
7. Settings > Dictation > General: "Open at login" is shown with its
   explanation; toggle it off and on, then check System Settings > General >
   Login Items lists Mouthpiece only while it's on.

## Pass criteria

No "...", "…", "--" or "—" from pauses in pasted text; sentences read
naturally (no stray period mid-sentence after "the"/"for"); spoken
ellipsis/em dash, CLI flags, hyphenated words and lists unchanged; the
login toggle matches System Settings.
