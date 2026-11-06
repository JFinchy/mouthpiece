# The mouthpiece command and MCP server (ask_user)

Branch: `agent-fleet`
Files changed: src-tauri/src/cli.rs, cli_server.rs, main.rs, recording.rs, WidgetView.tsx, settings/sections/agents.tsx, vocabulary.tsx

## What to verify

Agents and scripts can ask the user a question by voice and get the transcript back, and add vocabulary. Needs the built app's socket and a real MCP client.

## Steps

1. Launch the built app. `ls -l ~/Library/Application\ Support/dev.mouthpiece/bin/mouthpiece ~/Library/Application\ Support/dev.mouthpiece/mouthpiece.sock` shows the command (755) and socket (srw-------).
2. Run `~/Library/Application\ Support/dev.mouthpiece/bin/mouthpiece status`: JSON with version, state "idle", waiting agents.
3. Run `.../bin/mouthpiece listen --prompt "Which branch?"`. The widget shows "Agent asks: Which branch?" instead of the waveform. Say "use main", stop with your shortcut. The terminal prints "Use main." and nothing is pasted anywhere. History has it under "Agent question".
4. Run listen again and press Esc twice (Delete): the command prints "No answer: the recording was deleted…" and exits 1.
5. Run `.../bin/mouthpiece listen --timeout 5` and say nothing: after 5 s it exits 1 with "No answer within 5 s" and the recording is discarded.
6. Start a normal dictation, and while recording run listen: it refuses with "Mouthpiece is busy recording".
7. `.../bin/mouthpiece vocab add Hyperdrive` then open Settings > Vocabulary: Hyperdrive is listed (refreshes without reopening).
8. Copy the `claude mcp add …` command from Settings > Coding agents > Let agents ask you things, run it, start Claude Code and ask it to "use the ask_user tool to ask me what to name the branch". Answer by voice; Claude receives the text.
9. Quit Mouthpiece and run `mouthpiece status`: "Mouthpiece isn't running. Open it and try again." exit 1.

## Pass criteria

listen/ask_user return the spoken answer without pasting it, every failure path ends the command with a clear message, and the socket is user-only.
