# Mouthpiece

See `PRODUCT_SPEC.md` for the product vision, `FEATURES.md` for what's
built, `ROADMAP.md` for what's left, and `BACKLOG.md` for full detail
behind each backlog item (known issues, research findings, effort
estimates). `AUDIT.md` is the latest production-readiness review.

## Testing

`bun run test` runs typecheck, Rust tests and the Playwright UI e2e
suite; `bun run test:audio` runs real speech through Whisper. Details in
`TESTING.md`. Any dictation bug fix adds a case to
`src-tauri/tests/fixtures/pipeline_cases.json`; any new backend command
the UI calls needs a handler in `src/testing/mockBackend.ts`. `bun run
dev:mock` serves the UI in a browser without the Rust backend.

## Manual testing inbox

Work happens across several git worktrees at once (see `git worktree
list`), each on its own branch. `manual-testing-inbox/` is the
coordination point for merging them into `main` without losing track of
what still needs a human to click through it — full convention is
documented in `manual-testing-inbox/README.md`, summarized here:

- A branch that's implementation-complete (compiles, unit tests pass) but
  not yet manually exercised adds `manual-testing-inbox/<feature-slug>.md`
  as part of its normal diff, so the file merges into `main` along with
  the code it's testing. The file names its originating branch and gives
  concrete, copy-pasteable steps to verify the feature.
- Once several branches have merged, everything queued for a manual pass
  sits here together in `main` — work through each file against the
  merged build, not against the individual branch.
- Each inbox file also gets a line in `manual-testing-inbox/PRIORITIES.md`
  under Critical/Important/Minor, by impact if it were broken.
- **Pass** → move its `PRIORITIES.md` line to Backlog, dated. The detail
  file stays as the record of the verification; the permanent record of
  the feature is `FEATURES.md`.
- **Fail** → leave its line in its tier with a note on what broke; fix it
  on the originating worktree, re-merge, re-test.
- **Critical/Important/Minor all empty** = everything merged so far has
  been manually verified against current `main`.
