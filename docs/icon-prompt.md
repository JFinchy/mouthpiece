# App icon prompt

A prompt for an image model (written for Gemini) to explore app icon
directions for Mouthpiece. Paste everything inside the code block. When a
direction is picked, the 1024x1024 image becomes `src-tauri/icons/` (via
`bunx tauri icon <file>`) and `site/icon.png`, and the menu bar icon is
redrawn from it (`src-tauri/icons/source/tray-icon.py`).

```
Design a macOS app icon for "Mouthpiece", a free Mac app for software
developers. You press a hotkey, speak, and your words are typed at the
cursor as developer-ready text: file names become @file tags, "snake case
user id" becomes user_id, and replies go straight to AI coding agents like
Claude Code. Everything runs locally on the Mac; nothing goes to the cloud.

The name: a mouthpiece is the part you speak or play into (a brass
instrument's mouthpiece, a telephone's mouthpiece), and also someone who
speaks for you. The app is the mouthpiece between your voice and your code.
The feeling: crafted, precise, quietly confident. A well-made tool, not a
toy.

Requirements:
- macOS Big Sur-style rounded square (squircle), subtle depth, soft
  lighting. One bold, simple central symbol, no text or letters.
- Must still read at 16x16 in the menu bar and Dock: a strong silhouette,
  at most 2-3 shapes.
- Palette: deep graphite or ink-blue background, warm amber/brass
  (#E8A33D) as the main accent, optional soft violet (#A78BFA) as a
  secondary glow.
- Avoid clichés: no plain microphone, no generic sound-wave bars, no
  robot or brain, no chat bubble with "..." in it, no realistic lips or
  mouth.

Directions to explore (give me one icon for each, side by side):
1. A brass instrument mouthpiece seen in profile (the cup and narrow
   stem), polished amber metal, whose stem ends in a text cursor "|".
2. The mouthpiece seen head-on: a ring and its dark bore, read as an
   "O" of speech, with a small caret ">" glowing inside it.
3. A code chevron "<" that is also a funnel: sound goes in the wide end,
   a single clean line of text comes out the narrow end.
4. Your own idea that captures "the piece between your voice and your
   code".

Render each at 1024x1024 on a plain background, plus a 32px preview to
check that it reads at small sizes.
```
