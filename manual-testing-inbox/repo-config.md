# Per-repo .mouthpiece.json (team vocabulary, allowed replacements)

Branch: `agent-fleet`
Files changed: src-tauri/src/repo_config.rs, project.rs, pipeline.rs, config.rs, settings/sections/vocabulary.tsx, .mouthpiece.json

## What to verify

Trust gating for replacements from a cloned repo is a security boundary and needs checking against real files.

## Steps

1. This repo now has a .mouthpiece.json. Dictate once in a terminal inside it, then open Settings > Vocabulary: "Team settings from .mouthpiece.json: 14 vocabulary terms, used automatically."
2. Add `"replacements": [{"from": "post gress", "to": "Postgres"}]` to the file (keep it valid JSON). Wait a minute or dictate again so it re-reads. Settings shows the replacement "off until you allow them". Say "post gress": it is NOT replaced.
3. Click "Allow these replacements", say "post gress": you get "Postgres".
4. Edit the file (change "Postgres" to "PostgreSQL"). After it re-reads, the replacement is off again until allowed.
5. Add a personal replacement "post gress" -> "Postgres DB" in Vocabulary: the personal one wins.
6. Break the file (e.g. add `"snippets": []`): Settings shows the reason in amber and nothing from the file applies.

## Pass criteria

Vocabulary applies automatically; replacements apply only for the exact file you allowed; personal settings win; a bad file is explained, not applied.
