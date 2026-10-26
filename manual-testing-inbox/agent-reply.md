# Reply to a waiting coding agent by voice

Branch: `worktree-agent-acadcc1955e92915d`
Files changed: `src-tauri/src/agents.rs`, `agents/tests.rs`, `focus.rs`,
`resources/mouthpiece-hook`, `recording.rs`, `lib.rs`, `config.rs`,
`app_detect.rs`, `src/settings/sections/agents.tsx`, `src/WidgetView.tsx`

## What to verify

A Claude Code hook tells Mouthpiece an agent is waiting; a shortcut then
records a reply that is pasted into that agent's window and returns focus.
Process-tree lookup, Accessibility window raising, focus return and the
sound can only be checked on a real desktop with real terminals.

## Steps

1. `bun run tauri dev` (dev build). Open Settings > Coding agents.
2. Click "Copy hook config" and merge it into the `hooks` of
   `~/.claude/settings.json` (back the file up first). Confirm
   `ls -l ~/Library/Application\ Support/dev.mouthpiece/bin/mouthpiece-hook`
   exists and is `-rwx------` (it holds the install token).
3. Without Claude: `echo '{"hook_event_name":"Notification","session_id":"t1","cwd":"/tmp/demo","message":"Needs permission"}' | ~/Library/Application\ Support/dev.mouthpiece/bin/mouthpiece-hook`
   run from Terminal.app. Expect a Tink sound and the widget pill
   "Claude is waiting · demo"; `app.log` has `agents: waiting ... app=Some(("Terminal", pid))`.
4. Run `claude` in Terminal.app in a folder, ask it to run a shell command
   that needs permission. Switch to another app (e.g. Safari). The pill
   appears without the widget or Safari losing focus.
5. Click the pill: Terminal comes forward with the right window.
6. Back in Safari with a text field focused, press the reply shortcut
   (default ctrl+option+R), say "yes go ahead" (no "send it"), press it
   again. Expect: Terminal flashes forward, the text is pasted, focus
   returns to Safari (same window). Widget says "Sent to <folder> in
   Terminal". `app.log` has an `agents: reply hop` line with
   raise/paste/total ms.
6b. Same, but say "yes, go ahead. Send it." Unless the Terminal title
   shows the tty (Terminal > Settings > Profiles > Window > "TTY name"),
   Enter is not pressed: the text is pasted, Terminal stays in front,
   and the widget says "Pasted in Terminal. Check it's the <folder>
   agent's tab, then press Return." Turn on "TTY name" and repeat: Enter
   is pressed, Claude continues, focus returns to Safari.
6c. Say "fix the bug and send it": pasted as written, no Enter. Record a
   reply and say just "scratch that": widget says "Reply cancelled" and
   nothing is undone in Safari.
7. Repeat step 4-6 in VS Code and Cursor terminals (title contains folder),
   iTerm2, and Orca. Note whether the exact window was found or the
   widget said "can't pick the exact tab".
8. Two Terminal windows open, neither title naming the folder: reply
   should be copied to the clipboard and the widget says why.
9. Answer a prompt by typing in the terminal: pill disappears
   (UserPromptSubmit hook). Wait or simulate 30 minutes: pill expires.
10. Turn off the sound toggle; repeat step 3: no sound.
10b. `open -g "mouthpiece://agent-waiting?session=x&cwd=/tmp/fake"` (no
   token): no pill, and `app.log` has "ignoring a link without this
   install's token".
11. Press the reply shortcut with nothing waiting: widget error.
12. Set the reply shortcut equal to push-to-talk: rejected with a message.

## Pass criteria

Reply lands only in the agent's app, never elsewhere; focus returns to the
previous app and window; failures fall back to clipboard with a message;
the hook never slows or fails Claude Code.
