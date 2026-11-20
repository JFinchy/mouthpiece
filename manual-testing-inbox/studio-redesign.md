# Studio redesign: waveform studio, widget polish, mic check

Branch: `JFinchy/designer` (worktree `~/orca/workspaces/dev-whisper/designer`), plus review fixes on `release-0.6.2`
Files changed: see `git show --stat f60a02e`

## What to verify

The widget and Settings were redesigned: a new 60fps waveform with
engines, colourways and sliders, new fonts, a live mic check, and mode
previews. Automated tests cover the states, not how it looks, feels, or
how much CPU it uses on a real Mac.

## Steps

1. Install the 0.6.2 build. Dictate a few sentences in each widget style
   (Settings > Appearance). The waveform should follow your voice smoothly
   and settle when you stop. Pause (Esc), resume and transcribe still work.
2. Settings > Appearance: drag the Bars, Sensitivity, Snap Speed and Needle
   Width sliders. The preview follows the drag; the widget picks up the new
   look a moment after you let go. Quit and relaunch: the settings stuck.
3. Switch engines and colourways from Settings and from the widget's own
   menu. Both stay in sync.
4. Settings > Dictation: click **Test mic**. The meter moves with your voice
   and the macOS mic indicator (orange dot) is on. Click **Stop check**:
   the orange dot goes off within a second or two. Start it again and
   close Settings: the orange dot goes off.
5. Start a meeting and check the widget's meters for both sides. Stop it;
   the "Meeting transcript saved" pill still appears.
6. Drag the widget somewhere, relaunch: it comes back there. Switch to
   Notch style and back: the saved position is kept.
7. Settings > Modes: run a mode preview. It shows the result without
   pasting anything.
8. With Activity Monitor open, record for a minute: Mouthpiece's CPU stays
   reasonable, and drops back to near zero when idle.

## Pass criteria

All of the above behave as described; the mic indicator never stays on
after a mic check ends; fonts render (no fallback system font).
