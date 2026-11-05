# Hardened runtime on release builds

Branch: `main` (uncommitted, security audit 2026-11-05)
Files changed: `src-tauri/tauri.conf.json` (`hardenedRuntime: true`),
`src-tauri/Entitlements.plist` (comment only; still just `audio-input`),
`.github/workflows/release.yml` (verify step)

## What to verify

Release builds are now signed with the hardened runtime, so other code on
the Mac can't inject into Mouthpiece (DYLD_INSERT_LIBRARIES, debugger
attach) and use its Microphone/Accessibility/system-audio grants. The
runtime also blocks anything not explicitly entitled, and the failure mode
is silent: a missing entitlement means a microphone that records zeros, not
an error. Only a real signed bundle exercises this; `tauri dev` doesn't.

Pre-checked on the installed 0.5.3 bundle: the binary links only system
libraries (no `Frameworks/`, ONNX Runtime and whisper.cpp are static) and
has no JIT symbols, so no `disable-library-validation` or `allow-jit`
entitlement was added.

## Steps

1. Free disk space first (a release build needs several GB).
2. `cd src-tauri && bun run tauri build` (or push a tag and use the CI draft).
3. `codesign -dv --entitlements - target/release/bundle/macos/Mouthpiece.app`
   — `flags=0x10000(runtime)` and `com.apple.security.device.audio-input`.
4. Install it over the current app in `/Applications` and launch.
   Microphone and Accessibility should still be granted (with the release
   certificate the designated requirement is unchanged; an ad-hoc local
   build may need re-granting, which is expected).
5. Dictate a sentence into TextEdit — non-silent level meter, text pastes.
6. Fn double-tap toggles recording (event tap / Input Monitoring).
7. Isolated Voice on + a dictation (sherpa-onnx models load).
8. Pick a non-downloaded Whisper model, download it, dictate with it.
9. Start a meeting with system audio on during a call or a YouTube video —
   the system-audio channel is captured (process tap).
10. History lock: lock, unlock with Touch ID.
11. Agent reply: hook pill appears, reply shortcut hops and pastes.
12. LLM refinement with Ollama running (localhost network).
13. Auto-update: from this build, update to the next draft release
    (see `auto-update.md`) — it installs and relaunches.
14. Injection is blocked:
    `DYLD_INSERT_LIBRARIES=/tmp/x.dylib /Applications/Mouthpiece.app/Contents/MacOS/tauri-app`
    starts normally and `app.log` shows nothing odd (the variable is ignored).

## Pass criteria

All of 5–13 behave as on 0.5.3; step 3 shows the runtime flag and the
entitlement. If the mic records silence or a model fails to load, check
Console.app for `AMFI`/`Library Validation` lines and note them here.
