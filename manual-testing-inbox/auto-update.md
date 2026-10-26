# Auto-update (installed app picks up a new release)

Branch: `main` (shipped 2026-09-02 as part of v0.1.1, `ec4d6ee`)
Files changed: `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`,
`src-tauri/capabilities/default.json`, `src/SettingsView.tsx`,
`src/WidgetView.tsx`, `.github/workflows/release.yml`, `RELEASING.md`

## What to verify

An installed copy checks GitHub for a newer signed release at launch,
badges the widget's gear icon, and updates itself from Settings. Releases
v0.1.1 and v0.1.2 are published, but no installed copy has ever actually
updated, so the signature check, download, install and relaunch are all
unproven. Missed here, every future fix silently never reaches the
installed app.

## Steps

1. Quit the dev build (`tauri dev`) first: the installed app shares its
   config, hotkey and mic.
2. Download `Dev.Whisper_0.1.1_aarch64.dmg` from
   https://github.com/JFinchy/mouthpiece/releases/tag/v0.1.1, drag it to
   Applications, then right-click → Open (it's unsigned, so Gatekeeper
   asks once).
3. Within a few seconds of launch, the widget's gear icon shows an
   accent-colored dot. `app.log` shows `update: version 0.1.2 available`.
4. Settings → About: current version reads 0.1.1. Click **Check for
   updates**, then **Update to v0.1.2**. Progress shows, then the app
   relaunches (Gatekeeper may ask again, as with a fresh install).
5. Settings → About now reads 0.1.2, and the gear dot is gone.
6. Turn Wi-Fi off and relaunch: no error toast, no crash, just no badge.

## Pass criteria

Steps 3–5 complete without manual reinstalling, and step 6 fails
silently. If the update is rejected, note the exact `app.log` line
(likely a signature mismatch with `tauri.conf.json`'s `pubkey`).
