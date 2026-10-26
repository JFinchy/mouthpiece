# Discard with undo, and the new widget styles

Branch: `main` (committed directly, 2026-10-23)
Files changed: `src-tauri/src/discard.rs`, `capture.rs`, `recording.rs`,
`widget.rs`, `lib.rs`, `src/WidgetView.tsx`, `src/App.css`,
`src/settings/sections/dictation.tsx`

## What to verify

Global Esc/⇧Esc registration, real pasting after an undo, and the Notch
window's position only exist in the running app; UI tests cover the
widget against a fake backend.

## Steps

1. Start dictating in a text editor, say a few words, press Esc: the
   widget says "Recording discarded" with an Undo button; nothing is
   pasted. Within 8 seconds press ⇧Esc: it transcribes and pastes.
2. Discard again and wait 8 seconds: Undo disappears; ⇧Esc does nothing
   and reaches the frontmost app as normal.
3. While not recording, press Esc in another app (e.g. close a dialog):
   it works normally (Mouthpiece doesn't hold Esc).
4. Dictate a long sentence with an LLM mode and press Esc during
   "Refining": no paste, "Paste cancelled"; Undo pastes the text.
5. The ✕ on the widget while recording does the same as Esc.
6. Menu bar icon → Widget style: switch through all seven. The check
   mark follows; Settings → General shows the same choice; switching in
   the widget's hover menu (Style) updates both.
7. Notch: sits centered right under the menu bar (or notch) on the screen
   it's on; widens while recording and stays centered; returns to small
   when idle.
8. Live caption: words appear while you talk; after the paste the card
   shows what was pasted.

## Pass criteria

Discard never pastes, undo always pastes the right thing into the app you
were in, Esc is never stolen outside a recording, and every style is
legible with nothing clipped.
