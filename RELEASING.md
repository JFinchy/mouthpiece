# Cutting a release

One-time setup, then a 3-step checklist per release.

## One-time setup (already done on this machine, not yet on GitHub)

A minisign keypair for signing update packages lives at
`~/.tauri/dev-whisper.key` (private, password in the sibling
`~/.tauri/dev-whisper.key.password` file) and
`~/.tauri/dev-whisper.key.pub` (public — already embedded in
`tauri.conf.json`'s `plugins.updater.pubkey`). **Never commit the private
key or password file** — they're outside this repo on purpose.

The GitHub Actions release workflow (`.github/workflows/release.yml`)
needs that private key and password as repo secrets:

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/dev-whisper.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < ~/.tauri/dev-whisper.key.password
```

If this machine's private key is ever lost, generate a new keypair
(`bunx tauri signer generate -w ~/.tauri/dev-whisper.key`), update
`tauri.conf.json`'s `pubkey` with the new public key, and re-set both
secrets above — but note every already-installed copy of the app will
reject updates signed by the new key until it's manually reinstalled
once, since that's the whole point of the signature check.

### macOS signing certificate

Releases are signed with a self-made code-signing certificate, "Mouthpiece
Release Signing" (created 2026-10-27, valid to 2046). It isn't Apple's, so
Gatekeeper still shows its "can't verify" prompt, but every release
carries the same signer, and macOS keeps Accessibility and Microphone
permission across updates. (Ad-hoc signatures differ per build, so each
update used to silently lose them.)

The identity lives at `~/.tauri/mouthpiece-release-signing.p12`, its
password in the sibling `.p12.password` file, and the public certificate
in `.cert.pem`. **Back these up and never commit them.** The release
workflow reads them from two repo secrets:

```sh
base64 -i ~/.tauri/mouthpiece-release-signing.p12 | tr -d '\n' | gh secret set MACOS_SIGNING_P12
gh secret set MACOS_SIGNING_P12_PASSWORD < ~/.tauri/mouthpiece-release-signing.p12.password
```

Without the secrets the workflow builds ad-hoc signed; with them, its
verify step fails the release unless the app's designated requirement is
anchored to the certificate (`certificate leaf = H"da951adf…"`). If the
identity is ever lost, a new certificate works the same way, but every
installed copy asks for its permissions once more after that update.

## Per-release checklist

1. Bump the version everywhere it lives (`package.json`, `Cargo.toml`,
   `tauri.conf.json`; CI fails if they disagree):
   ```sh
   ./scripts/bump-version.sh 0.3.0
   ```
2. Commit that, then tag and push:
   ```sh
   git commit -am "Bump version to 0.3.0"
   git tag v0.3.0
   git push origin main v0.3.0
   ```
3. Watch the `Release` workflow run in the Actions tab. When it's green,
   go to the repo's Releases page and **publish the draft it created** —
   it's left as a draft on purpose (see the workflow's comment) so a bad
   build never silently becomes the "latest" version every installed
   copy auto-updates to. Publishing it is what makes the update actually
   visible to the updater endpoint.

Once published, any running copy of the app will see it on its next
launch (or immediately, if someone clicks "Check for updates" in
Settings → About).

The workflow also attaches `Mouthpiece.dmg`, a copy of the installer
under a fixed name. Once the release is published,
<https://github.com/JFinchy/mouthpiece/releases/latest/download/Mouthpiece.dmg>
downloads it; that's the link in the README.
