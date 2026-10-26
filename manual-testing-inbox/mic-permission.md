# Microphone permission in the built app

Branch: `main` (committed directly, 2026-10-23)
Files changed: `src-tauri/Info.plist`, `src-tauri/Entitlements.plist`,
`src-tauri/tauri.conf.json`, `src-tauri/src/mic_permission.rs`,
`readiness.rs`, `recording.rs`, `.github/workflows/release.yml`,
`src/settings/ReadinessPanel.tsx`, `readiness.ts`

## What to verify

The permission prompt and macOS's privacy database only exist on a real
Mac with an installed, bundled app; dev builds and tests can't show them.
Before 0.3.4 a built app was fed silence because Info.plist had no
`NSMicrophoneUsageDescription`.

## Steps

1. `/usr/libexec/PlistBuddy -c "Print NSMicrophoneUsageDescription" "/Applications/Mouthpiece.app/Contents/Info.plist"`
   prints the description.
2. Reset the grant to test the first-run path:
   `tccutil reset Microphone dev.mouthpiece`, then launch the app.
   macOS shows "Mouthpiece would like to access the microphone" with
   the description (at launch or on the first recording).
3. If you click Don't Allow: Settings shows "Microphone access is off"
   with Open System Settings, which opens Privacy & Security >
   Microphone. Recording shows "macOS is blocking the microphone…", not
   "Is it muted?".
4. Allow it there, return to Settings: the checklist item clears. Dictate:
   the level meter moves and text is transcribed.
5. Enterprise Mac with an MDM privacy profile that denies the mic: the
   checklist says "Microphone blocked by your organization".
6. After the next update (0.3.4 → later), check whether macOS asks again
   (ad-hoc signed apps can lose grants on update; note what happens).

## Pass criteria

A fresh install prompts for the microphone, recording works once allowed,
and every denied/blocked state names its cause and its fix.
