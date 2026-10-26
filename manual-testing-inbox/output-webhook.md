# Output webhook

Branch: `JFinchy/hippocamp`
Files changed: `src-tauri/src/webhook.rs` (new), `src-tauri/src/config.rs`,
`src-tauri/src/lib.rs`, `src-tauri/src/recording.rs`, `src/SettingsView.tsx`

## What to verify

After every delivered dictation, a configured URL gets a JSON `POST` with
`{timestamp_ms, text, summary, app_name, mode}` — the output-side
counterpart to the deep-link input hooks. Automated coverage
(`cargo test --lib webhook::`) checks the request body against a local
mock server and an unreachable-host error path, but nothing automated
exercises the real Tauri command wiring, the Settings UI, or a real
dictation end to end — that's what this pass is for.

## Steps

1. From repo root: `bun run tauri dev`. Look for the mic icon in the menu
   bar (not a window).
2. Open a browser tab to `webhook.site` and copy the unique URL it hands
   you.
3. Tray icon → **Open Settings…** → scroll to **Output webhook** (below
   History) → paste the URL → click elsewhere or press Enter to save.
4. Click **Send test event**. Confirm "Test event delivered." appears
   under the button within ~1s.
5. Switch to the webhook.site tab, confirm a new request landed, and
   check its JSON body matches:
   ```json
   {
     "timestamp_ms": <number>,
     "text": "This is a test event from Mouthpiece.",
     "summary": null,
     "app_name": null,
     "mode": "test"
   }
   ```
6. Leave the URL set. Trigger a real push-to-talk dictation into any app
   and let it paste. Check webhook.site for a second request with the
   real transcript in `text`, the frontmost app's name in `app_name`, and
   a resolved mode (`Plain`/`Casual`/`Cli`/`casing`/`boilerplate`/
   `snippet`) in `mode`.
7. Clear the URL field in Settings (save empty) and confirm a subsequent
   dictation does *not* produce a new webhook.site request — this is the
   off-by-default / opt-out path.
8. Spot-check Settings → **Logs**: a deliberately broken URL (e.g.
   `http://127.0.0.1:1`) should produce a `webhook: delivery to ... failed`
   or `webhook: test send to ... failed` line, and — critically — should
   **not** block or error the paste itself (dictation should still land
   normally even though the webhook failed).

## Pass criteria

Steps 4-8 all behave as described, with no delay added to the actual
paste at any point (webhook failures/timeouts must be invisible to the
core dictation flow).
