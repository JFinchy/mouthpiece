# Manual Testing: Meeting Mode Phase 2 (System Audio, Auto-Detection & Active-Speaker Naming)

Branch: `main` (uncommitted working tree, 2026-11-05; includes the bug-fix pass)

## Prerequisites
- macOS 14.4+ (for CoreAudio Process Tap `CATapDescription`)
- Accessibility permission granted to Mouthpiece (for active-speaker participant tile polling and browser call tab verification)

## 1. Local Call Auto-Detection & Auto-Stop Prompt
1. Open Settings → Meetings and confirm **"Offer to record when a call starts"** is enabled.
2. Join a Google Meet call in Chrome/Arc/Brave/Safari or start a Zoom / Microsoft Teams / Slack Huddle call.
3. Within ~6 seconds (3 consecutive 2s polls), verify the widget displays the **"Record Google Meet?"** (or **"Record Zoom?"**, etc.) pill.
4. Click the `✕` button on the pill and confirm it disappears and does not re-prompt during the same call.
5. Leave and rejoin a call, then click **"Record <App>?"** in the widget and verify meeting recording starts immediately.
6. End the call (close the Meet tab or leave the Zoom/Slack call). After ~10 seconds of call absence and system audio silence, verify the widget displays **"Call ended · Stop?"**. Click it to stop and transcribe.
7. **Muted mid-call:** record a call, mute yourself in the call app, and stay quiet with nobody talking for 20+ seconds. Verify **"Call ended · Stop?"** does *not* appear.
8. **Rejoin:** record a call, leave it, wait for **"Call ended · Stop?"**, then rejoin within a minute. Verify the pill disappears on its own, and appears again when you really leave.
9. **No false prompts:** with the browser using the mic for something else (e.g. a voice-search tab or a dictation site), make each of these the active tab in turn: a Google search for "how to join a zoom meeting", the Meet landing page (`meet.google.com`), and a Teams web *chat* (`teams.microsoft.com`, not in a call). Verify no **"Record …?"** pill appears for any of them.
   - Repeat the Teams chat case in Safari, and in Chrome with VoiceOver on (Cmd+F5). There, the page's controls are visible to Accessibility, so the Leave-button check is what rejects it. Turn VoiceOver off afterwards.
   - Then join a real Meet and a real Teams web call in Chrome *and* Safari, and verify the pill still appears for each within ~6 s.
10. Webex: start a call in the current Webex app and verify **"Record Webex?"** appears.

## 2. Dual-Channel Headphone Recording & Speakerphone Echo Gate
1. Put on headphones (AirPods or wired), start a meeting recording, and have a remote participant speak while you also reply.
2. Stop the meeting and wait for transcription to finish.
3. Open the transcript in Settings → Meetings:
   - Verify your speech is labelled **"You"** (from the left microphone channel, even without a voice profile enrolled).
   - Verify remote participants playing through your headphones are transcribed (from the right system audio tap channel).
4. Unplug headphones (using MacBook speakers + mic), play remote speech out loud while remaining quiet on the mic, and verify remote speech is not duplicated as **"You"** (suppressed by the cross-correlation Echo Gate).
5. **Bluetooth rate switch:** with AirPods connected and *playing* (48 kHz), start a meeting recording first, then join a call so the AirPods drop into headset mode. Have the remote side talk for a minute. In `app.log`, look for `system_audio: output format changed to … Hz; restarting tap`. Verify remote speech in the transcript is at normal speed and in sync with your replies.
6. **Backchannels vs. interjections:** on a headphone call, say "yeah" / "mm-hm" *over* a remote speaker a few times, and also answer once while they pause. Verify their speech stays in one paragraph across your "yeah"s with no missing words, and your answer in the pause appears between their paragraphs.
7. **In-room meeting, stray sound:** record an in-person conversation between two people at the laptop and let one notification sound play. Verify the transcript shows two speakers, not everyone as **"You"**.

## 3. Push-to-Talk Dictation Coexistence During Active Meetings
1. While a meeting is actively recording, focus a text editor or Slack message input and press the push-to-talk dictation shortcut.
2. Speak a short dictation sentence and release the shortcut.
3. Verify the dictated text pastes into the target app as usual, the meeting recording continues uninterrupted, and the private dictated sentence is silenced on the meeting's mic track.
4. In toggle mode, start a dictation during the meeting and click the widget's main button. Verify it stops the *dictation* (the text pastes) and the meeting keeps recording.
5. Verify the widget level meter doesn't flicker while dictating during a meeting.

## 5. 2.5-Hour Limit
Hard to wait out for real: temporarily set `MAX_DURATION` in `src-tauri/src/meeting.rs` to `Duration::from_secs(12 * 60)` and run `bun tauri dev`.
1. Turn on Settings → General → show the widget only while dictating, then start a meeting.
2. At 2 minutes in (10 minutes before the limit), verify the widget appears, flashes **"Meeting stops in 10 min (2.5-hour limit)…"**, and the timer turns amber, reading **"Ends in 9:59"** and counting down.
3. At the limit, verify the meeting stops, the widget says **"Meeting stopped at the 2.5-hour limit and is being transcribed. Start a new one to keep recording."**, and the transcript is saved.
4. Restore `MAX_DURATION`.

## 4. Offline Active-Speaker Naming via Accessibility (`AXUIElement`)
1. During a Google Meet, Slack Huddle, Microsoft Teams, or Zoom call where another participant speaks for at least 3 turns (or a >= 4s monologue), record the meeting with Accessibility permission enabled.
2. Stop the meeting and inspect the resulting transcript:
   - Verify the remote speaker's cluster is promoted from `"Speaker 1"` to their real display name from the conferencing UI tile.
   - Verify any participant who only flickered briefly or remained muted stays `"Speaker N"` rather than being misattributed.

## Pass criteria
Every numbered step behaves as described, with no false "Call ended" or "Record …?" prompts, and the transcript keeps your speech and remote speech in sync with nothing sped up or slowed down.
