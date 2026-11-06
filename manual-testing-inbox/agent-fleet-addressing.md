# Several agents at once: address by name, agent menu, Orca/iTerm/Terminal delivery

Branch: `agent-fleet`
Files changed: src-tauri/src/agents.rs, agents/address.rs, agents/targets.rs, widget_menu.rs, recording.rs, Entitlements.plist, Info.plist, WidgetView.tsx, settings/sections/agents.tsx

## What to verify

Dictations can be routed to a coding agent by its folder name, and replies reach the exact Orca pane or iTerm/Terminal tab. Needs real agents, real terminals and macOS permission prompts, none of which the tests can exercise.

## Steps

1. Set up the Claude Code hook (Settings > Coding agents > Connect an agent > Claude Code) and start two Claude Code sessions in different worktrees, e.g. `dev-whisper` and `dev-whisper-esc-pause`, in Orca.
2. Let both finish a turn. The widget pill reads "Claude is waiting · <newest folder> +1". Click it: a menu lists both, each with Reply by voice / Go to it / Dismiss.
3. In another app (e.g. Notes), press the dictation shortcut and say "To esc pause, say hello, send it." Stop. The text "Say hello" is submitted in the esc-pause agent's Orca pane and your window doesn't change.
4. Say "tell dev whisper to list the files" (no comma). It goes to the dev-whisper agent.
5. Say "To be honest, this is fine." in Notes. It pastes in Notes as spoken.
6. Settings > Coding agents > Talk to an agent by name lists "to dev whisper", "to dev whisper esc pause". Turn the toggle off and repeat step 3: it pastes in Notes instead.
7. Repeat steps 1-3 with two agents in iTerm2 tabs, then Terminal tabs. The first reply asks to let Mouthpiece control iTerm/Terminal (allow it). The reply lands in the right tab, Enter is pressed, and focus returns.
8. Deny the Automation prompt (System Settings > Privacy & Security > Automation, turn Mouthpiece off for Terminal) and reply again: it falls back to window titles (`agents: couldn't select the tab by tty` in app.log).

## Pass criteria

Routing by name works from any app, never fires on ordinary sentences, and Orca/iTerm/Terminal replies land in the agent's own pane/tab.
