# Rename to Mouthpiece (moving an existing install over)

Branch: `main`
Files changed: `src-tauri/src/rename.rs`, `src-tauri/src/agents.rs`,
`src-tauri/tauri.conf.json`, `site/install.sh`, the name everywhere else.

## What to verify

The app is now Mouthpiece, with a new bundle identifier (`dev.mouthpiece`).
Someone updating from Dev Whisper must keep their settings, history,
models and Claude Code hook, and end up with one app called Mouthpiece.
Only a real update from an installed 0.3.x shows this.

## Steps

1. With Dev Whisper 0.3.x installed and used (some History, a custom
   shortcut, Launch at login on, the Claude Code hook set up), quit it.
2. **Update in place:** open Dev Whisper, Settings → About → Check for
   updates, install 0.4.0. It relaunches.
3. `ls -ld ~/Library/Application\ Support/{com.devwhisper.app,dev.mouthpiece}`:
   `dev.mouthpiece` is a folder, `com.devwhisper.app` is a link to it.
4. `ls /Applications | grep -E "Dev Whisper|Mouthpiece"`: only
   `Mouthpiece.app`. (If App Management blocked the rename, it stays
   `Dev Whisper.app` and `grep rename: ~/Library/Application\ Support/dev.mouthpiece/app.log`
   says why. It still works.)
5. Settings: your shortcut, modes and vocabulary are there. History shows
   old entries.
6. `ls ~/Library/LaunchAgents | grep -E "Dev Whisper|Mouthpiece"`: only
   `Mouthpiece.plist`. Log out and in: Mouthpiece starts.
7. The setup checklist asks for Accessibility and Microphone again. Grant
   both, dictate into TextEdit: it pastes.
8. In Claude Code, make an agent wait for input: the "Claude is waiting"
   pill still appears (the old hook path in `~/.claude/settings.json`
   still works).
9. `open devwhisper://toggle-recording` and `open mouthpiece://toggle-recording`
   both start a recording.
10. **Fresh-install path:** with Dev Whisper installed, run
    `curl -fsSL https://jfinchy.github.io/mouthpiece/install.sh | sh`:
    it installs `Mouthpiece.app` and removes `Dev Whisper.app`; your
    settings carry over on first launch.

## Pass criteria

One app named Mouthpiece, with all earlier settings, history and models,
launch at login, the agent pill, and both link schemes working. The only
re-prompt is for macOS permissions.
