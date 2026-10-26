# App icon prompt

A prompt for an image model (written for Gemini) to explore new app icon
directions. Paste everything inside the code block. When a direction is
picked, the 1024x1024 image becomes `src-tauri/icons/` (via
`bunx tauri icon <file>`) and `site/icon.png`.

```
Design a macOS app icon for "Mouthpiece", a free Mac app for software
developers. You press a hotkey, speak, and your words are typed at the
cursor as developer-ready text: file names become @file tags, "snake case
user id" becomes user_id, and replies go straight to AI coding agents like
Claude Code. Everything runs locally on the Mac; nothing goes to the cloud.
The feeling: talking to your code, quietly and privately.

Requirements:
- macOS Big Sur-style rounded square (squircle), subtle depth, soft
  lighting. One bold, simple central symbol, no text or letters.
- Must still read at 16x16 in the menu bar and Dock: a strong silhouette,
  at most 2-3 shapes.
- Palette: deep graphite or ink-blue background, warm amber (#E8A33D) as
  the main accent, optional soft violet (#A78BFA) as a secondary glow.
- Avoid clichés: no plain microphone, no generic sound-wave bars, no
  robot or brain, no chat bubble with "..." in it.

Directions to explore (give me one icon for each, side by side):
1. A text cursor or terminal caret (">_" or "|") whose stroke turns into
   a single soft sound wave.
2. A code chevron "</>" drawn as one continuous, breath-like line.
3. A quiet "whisper" shape: a small curl of air or a closing quotation
   mark, hinting at a keystroke.
4. Your own idea that captures "voice becomes code".

Render each at 1024x1024 on a plain background, plus a 32px preview to
check that it reads at small sizes.
```

If the app is renamed (see "Rename the app" in `BACKLOG.md`), swap the
name in the first line; the rest still applies.
