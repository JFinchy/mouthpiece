# 0.6.1 release audit fixes

Branch: `release-audit`
Files changed: see `git log a3c88d8..release-audit`

## What to verify

A pre-release audit fixed bugs across the agent fleet, the `mouthpiece`
command, the dictation pipeline and Settings. The unit and UI tests cover
the logic; these steps check what only the real app can show: which app
is in front, real terminals and real recordings.

## Steps

1. **Enter is held when the agent has exited.** In iTerm or Terminal, start
   Claude Code in a repo, let it finish (the widget shows it), then quit
   Claude (Ctrl-C twice) so the tab is back at a shell prompt. Reply with
   the reply shortcut, saying "echo hello, send it". The text is pasted but
   **not** run, and the widget says to check the tab and press Return.
2. **Attach phrases stay out of prose.** In Mail or Slack, dictate "Could
   you attach the screenshot?" and "Haha, look at this." Both paste as
   spoken; no screenshot is taken. In a terminal, "fix this, attach the
   diff" still attaches the diff.
3. **Commit messages only on request.** In Claude Code, dictate "Write a
   commit message for these changes and commit them" with something staged:
   it pastes as spoken. "Commit message: fixed the race in paste" still
   writes one.
4. **Skip works while structuring.** Turn on Structure long prompts in a
   mode, dictate 90+ words, and press the widget's skip button while it's
   working: the dictation pastes as spoken within a second.
5. **`mouthpiece listen`.** Run `mouthpiece listen --prompt "Which branch?"`
   and press Ctrl-C while it's recording: the widget stops recording and
   the prompt goes away. Run it again, answer "scratch that": nothing in the
   front app is undone, and the command reports no answer.
6. **New vocabulary works at once.** `mouthpiece vocab add Zyxolotl`, then
   dictate the word without restarting: it's spelled right more often than
   before.
7. **Repo file Allow.** With a `.mouthpiece.json` that has replacements,
   dictate in that repo, then edit the file and open Settings > Vocabulary:
   the card shows the edited file, and Allow works.
8. **Reply diff.** Reply to an agent from a different app and say "attach
   the diff": the diff is the agent's repo's.

## Pass criteria

Every step behaves as described, with nothing run, pasted or captured that
the user didn't ask for.
