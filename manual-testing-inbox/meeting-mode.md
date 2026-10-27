# Meeting mode, phase 1 (mic only, transcript split by speaker)

Branch: `main`
Files changed: `src-tauri/src/meeting.rs`, `capture.rs`, `audio.rs`,
`lib.rs` (menu bar item), `src/settings/sections/meetings.tsx`,
`src/WidgetView.tsx`, `src-tauri/resources/pyannote_segmentation_3_0.onnx`.

## What to verify

Recording a real meeting through the real mic, and whether the speaker
split holds up with real voices in a real room. The automated tests only
use synthetic macOS voices fed straight in, never the mic.

## Steps

1. `bun run tauri dev` (or the next release). Settings → Meetings shows
   Start meeting, "People in the meeting", and the transcript list.
2. **Two people in the room:** set "People in the meeting" to 2, click
   Start meeting. The widget shows a red dot, level meter and
   "Meeting 0:05"-style timer; the menu bar item reads "Stop Meeting".
3. Talk with someone for 2-3 minutes, taking turns. While it records,
   press your dictation hotkey: nothing records (dictation waits).
4. Click the widget's record button to stop. The widget shows
   "Meeting: Finding who spoke…", then "Meeting: Transcribing N%", then
   "Meeting transcript saved". Dictate meanwhile: it works.
5. Settings → Meetings → View: each person's turns carry one consistent
   label, and the words are right. With your voice enrolled (Voice page),
   you're "You".
6. Repeat with "Work it out": note whether it still finds 2 people.
7. **Call on speakers:** start a meeting during a Zoom/Meet call on the
   laptop speakers. The other side shows up as a separate speaker.
8. **Crash safety:** start a meeting, talk 30 s, quit the app from the
   menu bar. Relaunch: the meeting is listed as "Not transcribed yet";
   Transcribe makes the transcript.
9. **Lock:** turn on the History lock: the Meetings list asks to unlock;
   Start meeting still works while locked.
10. `ls -l ~/Library/Application\ Support/dev.mouthpiece/meetings`: files
    are `-rw-------`, and no `.wav` is left after a transcript is saved.
11. Note how long an N-minute meeting took to transcribe (the log line
    `meeting: transcribed … in Ns`).

## Pass criteria

Each person gets one consistent label (with the count set), the words
are about as accurate as dictation, the widget and menu bar reflect the
meeting, nothing is lost on quit, and transcripts are owner-only and
behind the lock.
