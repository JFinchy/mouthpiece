# Allow pasting: copy instead of failing, fix from the widget

Branch: `main`
Files changed: `src-tauri/src/recording.rs`, `paste.rs`, `readiness.rs`,
`src/WidgetView.tsx`, `src/settings/ReadinessPanel.tsx`

## What to verify

Without Accessibility permission a dictation used to fail with a cut-off
red error and the words were lost. Now they're copied and the widget
offers a one-click fix. Needs the real macOS permission, which tests
can't toggle.

## Steps

1. System Settings > Privacy & Security > Accessibility: turn Mouthpiece
   off (or `tccutil reset Accessibility dev.mouthpiece`).
2. Dictate into TextEdit. Nothing is pasted; the widget shows
   "Copied · Allow pasting" in amber (Pill/Notch: "Allow pasting").
   Press ⌘V in TextEdit: your words are there. They're in History too.
3. Hover the widget's message: the tooltip explains ⌘V and the
   off-and-on-again case.
4. Click it: macOS's prompt appears and the Accessibility list opens with
   Mouthpiece in it. Turn it on. Within a couple of seconds the widget
   says "Pasting is on" and goes back to Ready, without reopening it.
5. Dictate again: it pastes.
6. Settings' setup checklist "Allow pasting" fix does the same (prompt,
   list, and the check turns green once granted).

## Pass criteria

No dictation is lost, the fix is one click from the widget, and the app
notices the grant by itself.
