# Dictation sounds

Branch: `main` (2026-11-04)
Files changed: `src-tauri/src/sounds.rs` (new), `src-tauri/src/config.rs`,
`src-tauri/src/recording.rs`, `src-tauri/src/lib.rs`,
`src/settings/sections/dictation.tsx`

## What to verify

An optional sound plays when recording starts, stops, and when the text is
pasted (or the dictation fails). Tests only check the setting is saved; a
human has to hear the sounds, judge the volume, and confirm the start sound
doesn't end up in the transcript.

## Steps

1. Settings > Dictation > General: check **Play sounds while dictating**.
   A soft "pop" plays right away as a preview.
2. Click into a text field, press the dictation shortcut: hear the pop.
3. Say a sentence, press the shortcut again: hear the "bottle" sound on
   stop, then the "glass" chime once the text is pasted.
4. Check the pasted text has no stray word at the start (the pop is
   played while the mic is opening, so it must not be transcribed).
   Try once with the laptop speakers rather than headphones.
5. Start a dictation and press Esc: hear the stop sound only (no chime).
6. Start a dictation, stay silent, stop: hear the stop sound, then the low
   "basso" error sound with the "no speech" message.
7. Uncheck the setting, dictate again: no sounds at all.
8. With sounds on, have a coding agent start waiting: its own "tink" still
   plays and is distinguishable from the dictation sounds.

## Pass criteria

Each cue plays exactly once at the right moment, at a comfortable volume,
nothing plays with the setting off, and the start sound never shows up as
text in a transcript.
