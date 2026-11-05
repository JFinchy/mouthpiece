# Esc pauses a dictation (Resume / Transcribe / Delete)

Branch: `esc-pause` (worktree `../dev-whisper-esc-pause`, 2026-11-05)
Files changed: `src-tauri/src/discard.rs`, `src-tauri/src/audio.rs`,
`src-tauri/src/recording.rs`, `src-tauri/src/lib.rs`, `src/WidgetView.tsx`,
`src/App.css`, `src/testing/mockBackend.ts`

## What to verify

Esc no longer throws a recording away with an easy-to-miss Undo: it pauses
and the widget asks what to do. Esc is also handed on to the app in front,
and only claimed while recording. The tests mock the backend, so the real
key forwarding, the paused audio and every widget style need a human.

## Steps

1. Widget style Compact. Click into a text field, press the dictation
   shortcut, say "first part", press Esc. The widget shows Resume /
   Transcribe / Delete; the meter stops moving.
2. While paused, say "this should not appear", then click **Resume**, say
   "second part", press the shortcut. Pasted text has "first part" and
   "second part" but not the sentence spoken while paused.
3. Dictate, press Esc, press the shortcut: it transcribes what you said.
4. Dictate, press Esc twice: nothing is pasted, the widget says "Recording
   deleted", no Undo button appears.
5. **Esc reaches other apps:** open a Finder window or a dialog that Esc
   closes (e.g. Safari's Find bar with ⌘F). Start a dictation, then press
   Esc with that window in front: the dictation pauses *and* the Find bar
   closes.
6. **Esc is free while transcribing:** turn on an LLM mode so refining
   takes a few seconds. Stop a dictation, and while it says Refining, press
   Esc in another app: the app gets it and the paste still lands.
7. Click the widget's ✕ while it's refining: nothing is pasted, the widget
   says "Paste cancelled · copied to clipboard", and ⌘V pastes the text.
8. Repeat step 1 in Pill, Notch, Live caption, Two-line, Detailed and
   Minimal. The controls fit without clipping (Pill/Notch show icons;
   hover them for the keys). In Minimal the dot turns amber and clicking
   it transcribes.
9. Pause for more than 3 seconds: no "No sound from mic" warning and no
   automatic mic switch.
10. With "Show the widget only while dictating" on, the widget stays up
    while paused.

## Pass criteria

Esc always pauses first and never loses audio by itself; Delete needs a
second Esc or the button. The app in front still receives every Esc, and
Esc is never claimed outside a recording. Nothing recorded while paused
ends up in the text.
