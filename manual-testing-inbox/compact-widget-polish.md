# Compact widget: wider, contained flyout, full error text

Branch: `main` (committed directly, 2026-10-23)
Files changed: `src/WidgetView.tsx`, `src/App.css`, `src-tauri/src/widget.rs`,
`src-tauri/tauri.conf.json`

## What to verify

Window sizing and macOS overlay scroll bars only show in the real
transparent window, not in the browser tests.

## Steps

1. Compact widget is 280px wide; "Transcribing 100%" and mic names fit.
2. Trigger the Accessibility error (turn off Accessibility for
   Mouthpiece, dictate): the widget grows to show the whole message, no "…".
3. Hover the widget with several mics connected: the flyout grows to at
   most 340px; longer lists scroll inside it with a thin scroll bar, and
   no big macOS scroll bar appears beside the widget.
4. Mic list shows a check on the active mic; Mic mode is one joined
   switch; Next dictation modes are two per row.

## Pass criteria

No clipped words, no scroll bar outside the widget, nothing overflowing.
