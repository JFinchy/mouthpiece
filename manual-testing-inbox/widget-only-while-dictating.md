# Show the widget only while dictating

Branch: `main` (2026-10-30)
Files changed: `src-tauri/src/widget.rs`, `src-tauri/src/config.rs`,
`src-tauri/src/recording.rs`, `src-tauri/src/meeting.rs`, `src-tauri/src/lib.rs`,
`src/WidgetView.tsx`, `src/settings/sections/dictation.tsx`

## What to verify

A setting that keeps the widget off screen except during a dictation:
it appears on the shortcut and hides itself after the paste. The e2e
tests only check that the widget asks to be hidden; whether the real
window appears and disappears, and never steals focus from the app being
dictated into, needs a human.

## Steps

1. Settings > Dictation > General: tick **Show the widget only while
   dictating**. The widget disappears within a second.
2. Click into a text field in another app (Notes). Press the dictation
   shortcut: the widget appears. Say a sentence, press the shortcut
   again. The words paste into Notes and the widget disappears about
   half a second later.
3. Press the shortcut and say nothing, then stop. "No speech detected"
   shows for about 6 seconds, then the widget hides.
4. Dictate again and, while it's transcribing, rest the pointer on the
   widget. After the paste it stays up; move the pointer away and it
   hides.
5. Dictate, then press Esc to discard. The widget stays while the undo
   offer is up, then hides.
6. Menu bar icon > **Show/Hide Widget**: the widget appears and stays.
   Dictate once: it hides after that paste.
7. Start a meeting from the menu bar: the widget appears for the
   meeting, and hides once it stops.
8. Quit and reopen Mouthpiece: no widget until the first dictation.
9. Untick the setting: the widget comes back and stays after dictations.

## Pass criteria

With the setting on, the widget is only on screen during a dictation or
meeting (plus the moments listed above), every paste still lands in the
app you were in, and with it off the widget behaves as before.
