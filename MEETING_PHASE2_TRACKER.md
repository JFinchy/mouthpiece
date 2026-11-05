# Meeting Mode Phase 2 — Implementation Tracker & Agent Handoff

> **Purpose:** Live implementation tracker for Meeting Mode Phase 2 (designed 2026-11-04 after reverse-engineering Wispr Flow Notetaker v1.6.606 with `morluto/rea`).
> **For Claude / AI Agents picking this up:** Read this file first. It tracks every milestone, file touched, architectural contract, and verification command. Update the checkboxes and `Current Status` section after completing each sub-task.

## Current Status
- **Active Milestone:** Complete (100% of Milestones 1–5 + two independent subagent code review passes implemented and verified).
- **Last Verified Test Run:**
  - `tsc -p tsconfig.json --noEmit` -> **0 errors**
  - `cargo test --lib` -> **0 compiler warnings**; **460/460 non-socket tests passed** (incl. `mic_usage` 2/2, `meeting_detect` 11/11, `speaker_poller` 5/5, `system_audio` 4/4, `capture` 13/13, `audio::tests` 20/20, `meeting::tests` 23/23). Note: the 4 `llm`/`webhook` localhost `127.0.0.1` loopback tests and Playwright `test:e2e` require localhost port binding outside the terminal sandbox (`bun run test` on host).

---

## Milestone Checklist

### Milestone 1: CoreAudio Process Holders & Local Meeting Auto-Detection
- [x] **1.1 Extend `src-tauri/src/mic_usage.rs`**:
  - Query `kAudioProcessPropertyBundleID` (`'pbid'`), `kAudioProcessPropertyPID` (`'ppid'`), `kAudioProcessPropertyIsRunningInput` (`'piri'`), and `kAudioProcessPropertyIsRunningOutput` (`'piro'`).
  - Add `pub struct AudioProcessHolder { pub pid: i32, pub bundle_id: Option<String>, pub is_input: bool, pub is_output: bool }`.
  - Add `pub fn active_audio_holders() -> Option<Vec<AudioProcessHolder>>` (excluding own PID).
  - Keep `pub fn other_processes_using_mic() -> Option<Vec<i32>>` working for `ambient.rs`.
  - Add unit test `active_audio_holders_succeeds_and_excludes_this_process`.
- [x] **1.2 Create `src-tauri/src/meeting_detect.rs`**:
  - Match known native call bundle IDs (`us.zoom.xos`, `com.microsoft.teams2`, `com.microsoft.teams`, `com.tinyspeck.slackmacgap`, `com.cisco.webexmeetingsapp`, `com.apple.FaceTime`, `com.hnc.Discord`).
  - Match browsers (`com.google.Chrome`, `company.thebrowser.Browser`, `com.brave.Browser`, `com.microsoft.edgemac`, `com.apple.Safari`, `org.mozilla.firefox`) when holding audio and verify active call via `AXUIElement` window titles / `AXURL` / `AXDocument` (`meet.google.com/`, `| Microsoft Teams`, `teams.microsoft.com`, `teams.live.com`, `zoom.us/wc`, `app.slack.com/huddle`).
  - Implement pure, unit-testable `AutoDetectTracker` state machine:
    - Requires `CONFIRM_START_POLLS = 3` consecutive positive polls before firing `PromptStart(DetectedCall)`.
    - Tracks dismissed call identity so dismissing a prompt doesn't re-prompt during the same call.
    - Tracks active recording call binding and fires `PromptStop(DetectedCall)` when the call app releases mic+output (or browser tab closes) and system audio is silent for `CONFIRM_STOP_POLLS = 5` polls (~10s).
  - Unit tests for native app detection, browser call title/URL matching, confirm streak, dismiss latch, and auto-stop trigger.

### Milestone 2: Offline Active-Speaker Naming via Accessibility (`AXUIElement`)
- [x] **2.1 Create `src-tauri/src/speaker_poller.rs`**:
  - Define `pub struct SpeakerObservation { pub offset_ms: u64, pub speaking_names: Vec<String>, pub muted_names: Vec<String> }`.
  - Implement two-tier anchored AX polling (`AnchoredSession` discovery every 5s + fast 300ms compact poll) for:
    - **Google Meet**: `AXDOMClassList` tile root `"oZRSLe"`, active speaker marker `"kssMZb"`, plus semantic AX fallback (`"speaking"`, `"muted"`).
    - **Slack Huddles**: `AXDOMClassList` tile root `"p-huddle_peer_tile"`, active marker `"p-huddle_peer_tile__overlay--active_speaker"`.
    - **Microsoft Teams**: `"vdi-frame-occlusion"` + semantic tile `AXTitle`/`AXDescription` (`"speaking"`, `"talking"`, `"active speaker"`, `"muted"`).
    - **Zoom**: native Cocoa window/tile `AXTitle`/`AXDescription`/`AXValue` inspection.
  - Set `AXEnhancedUserInterface = true` on Chromium/Electron PIDs when starting discovery.
- [x] **2.2 Implement Post-Call Cluster-to-Name Fusion in `src-tauri/src/meeting.rs`**:
  - Add `pub fn resolve_cluster_names(turns: &[Turn], observations: &[SpeakerObservation], you: Option<i32>) -> HashMap<i32, String>`.
  - Compute single-speaker temporal overlap per `(cluster_id, participant_name)`, apply `0.5x` penalty when a participant is in `muted_names` for `>50%` of a turn, and require `SPEAKER_PROMOTE_TURNS = 3` corroborated turns (or `>= 4.0s` unambiguous overlap) + `>= 0.30` overlap ratio to promote a cluster from `"Speaker N"` to the participant's real name.
  - Unit tests in `meeting.rs` covering promotion after 3 turns, single-turn flicker rejection, muted-speaker negative evidence, and multi-speaker assignment.

### Milestone 3: Dual-Channel System Audio Capture (`CATapDescription`) + Echo Gate
- [x] **3.1 Add `NSAudioCaptureUsageDescription` to `src-tauri/Info.plist`**.
- [x] **3.2 Create `src-tauri/src/system_audio.rs`**:
  - macOS 14.2+ (gated at `>= 14.4`) CoreAudio Process Tap (`CATapDescription` excluding own PID + `AudioHardwareCreateProcessTap` + `AudioHardwareCreateAggregateDevice` + IOProc callback sending `AudioChunk`s).
  - Automatic stalled-IOProc watchdog recovery (`should_restart_tap` every 3s, recreating the tap after 2 consecutive stalled ticks on audio route/device changes).
  - Graceful fallback to mic-only if permission is denied or unavailable.
- [x] **3.3 Stereo WAV + Echo Gate + Channel-Split Diarization in `src-tauri/src/meeting.rs`**:
  - Record 2-channel 16 kHz WAV (`Left = Mic`, `Right = System`) when system audio tap is active; maintain 1-channel WAV support for mic-only fallback and existing tests.
  - Implement `pub fn suppress_speakerphone_echo(mic: &mut [f32], system: &[f32])` (sliding 20ms cross-correlation echo gate across `0..120ms` lag with attack walkback and hangover trim) + unit tests.
  - Implement stereo diarization: Channel 0 (`mic` after echo gate) -> `"You"` segments; Channel 1 (`system`) -> `pyannote` + `WeSpeaker` `group_voices` + `resolve_cluster_names`.

### Milestone 4: Push-to-Talk Coexistence During Active Meetings
- [x] **4.1 Update `src-tauri/src/capture.rs` & `src-tauri/src/audio.rs`**:
  - Allow `StartDictation` / `StopDictation` / `ToggleDictation` / `DiscardDictation` while a meeting is recording (`Sinks::deliver` feeds zeroed frames of the same length to the meeting tap while push-to-talk dictation buffers, keeping stereo sample-clock sync intact).
  - Update unit tests in `capture.rs` and `audio.rs`.

### Milestone 5: UI Integration, Mock Backend & Verification
- [x] **5.1 Wire auto-detect/auto-stop prompts & settings into `src/WidgetView.tsx` (across all widget styles incl. `pill`, `compact`, `stacked`, `notch`, `caption`, and `detailed`), `src/settings/sections/meetings.tsx`, `src/testing/mockBackend.ts`, and `e2e/meetings.spec.ts`**.
- [x] **5.2 Add `manual-testing-inbox/meeting-phase2.md` and update `manual-testing-inbox/PRIORITIES.md`, `FEATURES.md`, `ROADMAP.md`, and `BACKLOG.md`**.
- [x] **5.3 Final full-repo `cargo test --lib` + `tsc --noEmit` verification** (`460/460` non-socket Rust unit tests passed, `0` compiler warnings, `0` TypeScript errors; run `bun run test:e2e -- e2e/meetings.spec.ts` on host where localhost port binding is allowed).
- [x] **5.4 Subagent Code Review & Hardening (Pass 1 & Pass 2)**:
  - Fixed `write_meeting_stereo` 100ms `To16k` block-boundary jitter and channel draining (`JITTER_TOLERANCE = 200ms`, `MAX_SYS_LOOKAHEAD = 500ms`) so staggered callback arrivals never splice zero-gaps or shift stereo phase.
  - Separated `TEAMS_TILE_ROOT_CLASSES` (`"vdi-frame-occlusion"`) from active-speaker indicator matching in `speaker_poller.rs` so Teams tiles require a semantic speaking marker.
  - Preserved widget `"recording"` phase and visibility after completing or discarding a mid-meeting push-to-talk dictation in `src/WidgetView.tsx`, tracked `dictating` state separately from `meetingRecording` (for timer, status label, discard button, and level meter), suppressed the 340px settings flyout while meeting prompt pills are shown, added `minimal` mode call rings/actions, and stopped `onMouseDown` drag propagation on `meetingDetectPill` and `meetingEndedPill`.
  - Mapped Safari (`com.apple.WebKit.GPU`) and `com.apple.avconferenced` to their parent app bundles in `canonical_bundle_id`, supported `CFURLRef` for `AXURL`, latched `AutoDetectTracker::dismissed` when manually stopping a recording mid-call, merged dual-channel segments chronologically before `merge_turns` so turns never merge across interjections on the other channel, re-numbered unpromoted `"Speaker N"` clusters sequentially, and passed `remote_speakers = (speakers - 1).max(1)` in the dual-channel diarization path.
