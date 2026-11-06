# Hooks for Codex, Gemini CLI, OpenCode and Aider

Branch: `agent-fleet`
Files changed: src-tauri/resources/mouthpiece-hook, agents.rs, settings/sections/agents.tsx

## What to verify

The hook script now serves five agents with different calling conventions; the tests run the script with sample payloads, but not the agents themselves.

## Steps

1. Settings > Coding agents > Connect an agent: each of Claude Code, Codex, Gemini CLI, OpenCode, Aider shows a snippet and a file.
2. Codex: add the `notify = [...]` line to ~/.codex/config.toml, run `codex`, let it finish a turn. The pill says "Codex is waiting · <folder>".
3. Gemini CLI: merge the hooks JSON into ~/.gemini/settings.json, run `gemini`, finish a turn ("Gemini is waiting"), then type a prompt: the pill clears (BeforeAgent). Check Gemini prints no hook warning.
4. OpenCode: save the plugin to ~/.config/opencode/plugins/mouthpiece.js, restart opencode, finish a turn: "OpenCode is waiting".
5. Aider: add the two lines to ~/.aider.conf.yml, run `aider`, let it wait for input: "Aider is waiting". Aider must not hang (the script never reads the terminal).
6. For each, reply with the reply shortcut and check it lands in that agent.

## Pass criteria

Each agent shows up in the pill with its own name and can be replied to; no agent errors or hangs because of the hook.
