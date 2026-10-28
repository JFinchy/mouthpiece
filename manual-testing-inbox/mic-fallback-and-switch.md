# Mic fallback, dead-mic recovery, Switch mic, decibel level meter, widget menu

Branch: `main` (2026-10-28)
Files changed: `src-tauri/src/input_devices.rs` (new), `src-tauri/src/audio.rs`,
`src-tauri/src/recording.rs`, `src-tauri/src/meeting.rs`, `src-tauri/src/lib.rs`,
`src/WidgetView.tsx`

## What to verify

Real Bluetooth hardware is the whole point here: AirPods that go silent
once music plays, a missing picked mic, switching streams mid-recording.
Unit tests cover the choices and the format conversion, not the devices.

## Steps

1. **Level meter.** Dictate at a normal voice, then quietly, then loudly.
   The bars should sit around half height for normal speech, visibly lower
   for quiet, near the top only when loud. A quiet room shows flat bars.
2. **Fallback skips Bluetooth.** With a picked mic that isn't connected
   (e.g. AIRHUG 28 unplugged) and AirPods connected as the system default,
   relaunch. `app.log` shows `falling back to MacBook Pro Microphone` and
   dictation works from the laptop mic; music on the AirPods stays at full
   quality while dictating.
3. **Dead mic recovered mid-recording.** In Settings → Microphone pick
   "System default" with AirPods as the default, play music on the AirPods,
   then dictate. Within ~2 s the widget shows "Mic switched" and the
   words are transcribed. `app.log`: `audio: switched mid-recording from
   Jake’s AirPods #2 to 'MacBook Pro Microphone'`. The next dictation stays
   on the MacBook mic.
4. **Switch mic menu.** Pick the AirPods explicitly, play music, dictate.
   The widget shows **Switch mic** (no automatic switch: you picked them).
   Click it: a menu lists the MacBook mic first ("(this Mac)"), the
   AirPods ticked, then System default. Pick the MacBook mic: the
   recording carries on and transcribes. Settings now shows the MacBook
   mic as chosen.
5. **After a silent recording.** Same as 4 but stop without switching: the
   error offers **Switch mic** too.
6. Try steps 3-4 in the pill style as well as compact.
7. **Widget menu.** In each style (switch via the menu itself), right-click
   the widget: a native menu with Microphone, Mic mode, Next dictation,
   Widget style and Open Settings… appears next to the cursor, even in the
   notch and the Minimal dot. In Pill, Notch (while active), Caption and
   Detailed, hovering also shows a ⌄ button that opens it. Each choice
   takes effect (mic in Settings, Voice Isolation in Compact's hover panel,
   the next mode used once, the style changes).

## Pass criteria

Speech is never lost to a silent Bluetooth mic you didn't pick; a silent
mic you did pick is one click from a working one; the meter shows a range,
not just full bars.
