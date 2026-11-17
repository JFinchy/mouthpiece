# Meetings: stand-up speakers, saved-meeting handoff, rename and summary

Branch: `meeting-phase2`
Files changed: see `git show fc01732 --stat`

## What to verify

Phase 2's later milestones (5.5-5.7 in `MEETING_PHASE2_TRACKER.md`): telling
apart many voices on a call, what happens right after a meeting is saved,
and editing the saved notes. The audio tests use synthetic voices; real
calls, real windows and the local model need a person.

## Steps

1. **Stand-up.** Join a real call with 4+ people (Zoom, Meet or Teams) and
   record it with the speaker count on "Work it out". In the saved notes,
   each person mostly has their own label; voices it couldn't separate show
   as "Group", never as one "Speaker 1" for everyone.
2. **Saved pill.** Stop the meeting. The widget shows "Meeting transcript
   saved · View" and "Copy" in every widget style (switch styles in the
   flyout and record a short meeting in two or three of them). Copy puts the
   transcript on the clipboard. View opens Settings > Meetings with that
   meeting open and scrolled into view.
3. **Menus.** With Settings closed, choose "View Meeting Notes…" from the
   menu bar icon, then from the widget's right-click menu. Both open the
   newest meeting. Quit Settings, choose it again: Settings opens straight
   to it (including in the Chain layout).
4. **Rename.** In the open meeting, rename "Speaker 1" to a name: every
   line by that speaker changes, and "Speaker 10" (if any) doesn't. Rename
   the title. Reopen Settings: both stuck.
5. **Summary.** With a local model installed, click the summary button: a
   summary appears within a minute and is saved with the notes. With
   Ollama stopped, it shows an error rather than spinning forever.
6. **History lock.** Lock history in Settings > Privacy; the Meetings list
   in another open Settings section reflects the lock at once.

## Pass criteria

Each step behaves as described; no meeting file is lost or garbled by
renaming or summarising.
