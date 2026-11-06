use rdev::{simulate, EventType, Key};
use std::{thread, time::Duration};

const PASTE_SETTLE_BEFORE_ENTER: Duration = Duration::from_millis(150);

#[cfg(target_os = "macos")]
pub fn accessibility_trusted() -> bool {
    macos_accessibility_client::accessibility::application_is_trusted()
}

#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted() -> bool {
    true
}

/// Copies `text` to the clipboard without simulating a paste keystroke —
/// used by the "copy only" setting, and doesn't need Accessibility
/// permission since it never sends a synthetic keystroke.
pub fn copy_text(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// Copies `text` to the clipboard and simulates Cmd+V so it lands at the
/// active cursor position. Requires macOS Accessibility permission. Without
/// it, `simulate()` below returns Ok but the OS silently drops the
/// synthetic keystroke — checking first turns that into a visible error
/// instead of a paste that mysteriously never happens.
pub fn paste_text(text: &str) -> Result<(), String> {
    if !accessibility_trusted() {
        return Err(
            "Accessibility permission not granted. Enable Mouthpiece in System Settings > \
             Privacy & Security > Accessibility, then try again."
                .to_string(),
        );
    }

    copy_text(text)?;

    // Read the clipboard back and confirm it holds this text before sending
    // Cmd+V, re-issuing the write if not, rather than trusting one fixed
    // sleep for the pasteboard write to land. Defensive: the stale-looking
    // terminal pastes this was first added for turned out to be the old CLI
    // LLM prompt rewriting dictation into shell commands, not a clipboard
    // race.
    let mut verified = false;
    for attempt in 0..5 {
        thread::sleep(Duration::from_millis(if attempt == 0 { 50 } else { 30 }));
        let matches = arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .map(|current| current == text)
            .unwrap_or(false);
        if matches {
            verified = true;
            break;
        }
        let _ = copy_text(text);
    }
    if !verified {
        crate::applog!(
            "paste: clipboard still didn't match the dictated text after retrying, \
             pasting anyway (may deliver stale clipboard content)"
        );
    }

    command_chord(Chord::Paste)
}

/// Simulates pressing Enter — used by the "press enter" voice command
/// (`punctuation::extract_press_enter`) after the transcript itself has
/// already been delivered. Requires Accessibility permission, same as
/// `paste_text`, since it's also a synthetic keystroke.
pub fn press_enter() -> Result<(), String> {
    if !accessibility_trusted() {
        return Err(
            "Accessibility permission not granted. Enable Mouthpiece in System Settings > \
             Privacy & Security > Accessibility, then try again."
                .to_string(),
        );
    }

    // Electron/xterm.js-based terminals (VS Code, Orca, Hyper) and Warp
    // handle a paste asynchronously; a Return sent right behind Cmd+V can be
    // processed first and submit an empty or partial line.
    thread::sleep(PASTE_SETTLE_BEFORE_ENTER);
    send(EventType::KeyPress(Key::Return))?;
    send(EventType::KeyRelease(Key::Return))?;
    Ok(())
}

fn require_accessibility() -> Result<(), String> {
    if accessibility_trusted() {
        Ok(())
    } else {
        Err(
            "Accessibility permission not granted. Enable Mouthpiece in System Settings > \
             Privacy & Security > Accessibility, then try again."
                .to_string(),
        )
    }
}

/// Simulates Cmd+Z, for "scratch that" in apps with a real undo stack (see
/// `undo.rs` for when this is allowed).
pub fn send_undo() -> Result<(), String> {
    require_accessibility()?;
    command_chord(Chord::Undo)
}

#[derive(Clone, Copy)]
enum Chord {
    Paste,
    Undo,
    #[cfg(test)]
    SelectAll,
    #[cfg(test)]
    Copy,
}

/// Pause between the chord's events. The flags make the order of events
/// the only thing that matters, so this is just politeness to slow apps.
const CHORD_GAP: Duration = Duration::from_millis(15);

/// Cmd+V / Cmd+Z with the Command flag set on the letter's own events.
///
/// rdev builds each key event from the OS's *current* modifier state, so a
/// separately posted Cmd keydown only counts if macOS has absorbed it by
/// the time the V is built. When the window server stalls (GPU work from
/// Whisper or an LLM, another app's keyboard tap) that takes longer than
/// any fixed sleep, and the app receives a bare "v" while the transcript
/// sits on the clipboard (`probe_modifier_race_in_the_old_paste` measures
/// it). Events from a private source with explicit flags don't depend on
/// timing, or on modifiers the user is physically holding.
#[cfg(target_os = "macos")]
fn command_chord(chord: Chord) -> Result<(), String> {
    command_chord_with_gap(chord, CHORD_GAP)
}

#[cfg(target_os = "macos")]
fn command_chord_with_gap(chord: Chord, gap: Duration) -> Result<(), String> {
    use core_graphics::event::CGEventTapLocation;
    for event in chord_events(chord)? {
        event.post(CGEventTapLocation::HID);
        thread::sleep(gap);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
const KEY_COMMAND: u16 = 0x37;

/// Command plus the device bit for the *left* Cmd key (`NX_DEVICELCMDKEYMASK`),
/// which a real keyboard sets and which VMs, remote desktops and remappers
/// look at.
#[cfg(target_os = "macos")]
fn command_flags() -> core_graphics::event::CGEventFlags {
    core_graphics::event::CGEventFlags::CGEventFlagCommand
        | core_graphics::event::CGEventFlags::from_bits_retain(0x08)
}

#[cfg(target_os = "macos")]
fn chord_events(chord: Chord) -> Result<Vec<core_graphics::event::CGEvent>, String> {
    use core_graphics::event::{CGEvent, CGEventFlags};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // kVK_ANSI_V / kVK_ANSI_Z: physical key positions, like rdev's.
    let letter: u16 = match chord {
        Chord::Paste => 0x09,
        Chord::Undo => 0x06,
        #[cfg(test)]
        Chord::SelectAll => 0x00,
        #[cfg(test)]
        Chord::Copy => 0x08,
    };
    let command = command_flags();
    // The Cmd key events are kept for apps that track modifier state
    // themselves; the letter carries the flag either way.
    [
        (KEY_COMMAND, true, command),
        (letter, true, command),
        (letter, false, command),
        (KEY_COMMAND, false, CGEventFlags::CGEventFlagNull),
    ]
    .into_iter()
    .map(|(key, down, flags)| {
        let source = CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|()| "couldn't create a keyboard event source".to_string())?;
        let event = CGEvent::new_keyboard_event(source, key, down)
            .map_err(|()| "couldn't create a keyboard event".to_string())?;
        event.set_flags(flags);
        Ok(event)
    })
    .collect()
}

#[cfg(not(target_os = "macos"))]
fn command_chord(chord: Chord) -> Result<(), String> {
    let letter = match chord {
        Chord::Paste => Key::KeyV,
        Chord::Undo => Key::KeyZ,
        #[cfg(test)]
        Chord::SelectAll => Key::KeyA,
        #[cfg(test)]
        Chord::Copy => Key::KeyC,
    };
    send(EventType::KeyPress(Key::ControlLeft))?;
    thread::sleep(Duration::from_millis(40));
    send(EventType::KeyPress(letter))?;
    send(EventType::KeyRelease(letter))?;
    send(EventType::KeyRelease(Key::ControlLeft))?;
    Ok(())
}

/// Simulates exactly `count` Backspace presses, for "scratch that" in
/// terminals: deletes what was pasted and nothing else.
pub fn send_backspaces(count: usize) -> Result<(), String> {
    require_accessibility()?;
    // Tighter than `send`'s 20ms gap, which would take seconds for a long
    // line; the count is capped well below where this could drop events.
    for _ in 0..count {
        simulate(&EventType::KeyPress(Key::Backspace)).map_err(|e| format!("{e:?}"))?;
        simulate(&EventType::KeyRelease(Key::Backspace)).map_err(|e| format!("{e:?}"))?;
        thread::sleep(Duration::from_millis(4));
    }
    Ok(())
}

fn send(event: EventType) -> Result<(), String> {
    simulate(&event).map_err(|e| format!("{e:?}"))?;
    thread::sleep(Duration::from_millis(20));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression for the bare "v": the letter must carry Command by
    /// itself, built while no Cmd is held at all (as when the OS hasn't yet
    /// absorbed the Cmd keydown). Building events needs no permission.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_paste_letter_carries_command_without_cmd_being_held() {
        use core_graphics::event::{CGEventFlags, EventField};

        for (chord, letter) in [(Chord::Paste, 0x09), (Chord::Undo, 0x06)] {
            let events = chord_events(chord).unwrap();
            let keys: Vec<(i64, CGEventFlags)> = events
                .iter()
                .map(|e| {
                    (
                        e.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
                        e.get_flags(),
                    )
                })
                .collect();
            let command = CGEventFlags::CGEventFlagCommand;
            let others = CGEventFlags::CGEventFlagShift
                | CGEventFlags::CGEventFlagControl
                | CGEventFlags::CGEventFlagAlternate
                | CGEventFlags::CGEventFlagSecondaryFn;
            assert_eq!(keys.len(), 4);
            assert_eq!(keys[0].0, KEY_COMMAND as i64);
            assert_eq!(keys[3].0, KEY_COMMAND as i64);
            for (key, flags) in &keys[..3] {
                assert!(flags.contains(command_flags()), "Cmd held on key {key}");
                // Only Command: a held Option or Shift would turn Cmd+V
                // into "paste and match style" or something else entirely.
                assert!(!flags.intersects(others), "stray modifier on key {key}");
            }
            assert_eq!((keys[1].0, keys[2].0), (letter, letter));
            assert!(!keys[3].1.contains(command), "Cmd is released at the end");
        }
    }

    /// Exercises the real system clipboard rather than mocking it — save
    /// and restore whatever was already there so the test doesn't clobber
    /// the developer's actual clipboard contents.
    #[test]
    fn copy_text_writes_to_the_real_clipboard() {
        let mut clipboard = arboard::Clipboard::new().expect("clipboard unavailable");
        let previous = clipboard.get_text().ok();

        let marker = "dev-whisper-copy-only-test-marker";
        copy_text(marker).expect("copy_text failed");
        // The pasteboard is shared with every other app (clipboard
        // managers re-read and sometimes re-write it), so allow the write
        // a moment to settle rather than asserting on the first read.
        let landed = (0..20).any(|_| {
            let ok = clipboard.get_text().is_ok_and(|t| t == marker);
            if !ok {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            ok
        });
        assert!(landed, "clipboard never showed the copied text");

        if let Some(previous) = previous {
            let _ = clipboard.set_text(previous);
        }
    }

    /// Measures the race behind "a bare v instead of the paste": rdev
    /// creates every key event from the HID system state, so the V carries
    /// a modifier only if the OS has already absorbed the separately posted
    /// modifier keydown. Taps Shift rather than Cmd (same mechanism, harmless
    /// if someone is typing) and never posts the V it builds. Needs
    /// Accessibility for the terminal running it.
    ///
    /// `cargo test --lib paste::tests::probe -- --ignored --nocapture`,
    /// with `PROBE_LOAD=<threads>` to add CPU load like a running Whisper.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "taps Shift on the real keyboard; needs Accessibility"]
    fn probe_modifier_race_in_the_old_paste() {
        use core_graphics::event::{CGEvent, CGEventFlags};
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        assert!(
            accessibility_trusted(),
            "grant the terminal Accessibility first"
        );
        let stop = Arc::new(AtomicBool::new(false));
        let load: usize = std::env::var("PROBE_LOAD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let busy: Vec<_> = (0..load)
            .map(|_| {
                let stop = stop.clone();
                thread::spawn(move || {
                    let mut x = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    x
                })
            })
            .collect();

        let rounds = 40;
        for delay_ms in [0u64, 2, 10, 40, 150] {
            let mut missing = 0;
            for _ in 0..rounds {
                simulate(&EventType::KeyPress(Key::ShiftLeft)).unwrap();
                thread::sleep(Duration::from_millis(delay_ms));
                // Exactly how rdev builds the V keydown.
                let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).unwrap();
                let v = CGEvent::new_keyboard_event(source, 9, true).unwrap();
                if !v.get_flags().contains(CGEventFlags::CGEventFlagShift) {
                    missing += 1;
                }
                simulate(&EventType::KeyRelease(Key::ShiftLeft)).unwrap();
                thread::sleep(Duration::from_millis(30));
            }
            eprintln!(
                "load={load} delay={delay_ms:>3}ms: V without the modifier {missing}/{rounds}"
            );
        }
        stop.store(true, Ordering::Relaxed);
        for t in busy {
            let _ = t.join();
        }
    }

    /// End to end: pastes numbered tokens into a fresh TextEdit document,
    /// then reads the document back and counts pastes that arrived as a
    /// bare "v". Runs the old rdev chord and the new one side by side.
    /// Takes over the keyboard for about a minute and stops the moment
    /// TextEdit isn't frontmost, so it never types anywhere else. Needs
    /// Accessibility for the terminal.
    ///
    /// `cargo test --lib paste::tests::stress -- --ignored --nocapture`
    /// Env: `PASTE_ROUNDS` (default 60), `PASTE_GAP_MS` (gap between the
    /// chord's events; the old paste used 40 after Cmd), `PROBE_LOAD`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "drives TextEdit with real keystrokes; needs Accessibility"]
    fn stress_paste_into_textedit() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        assert!(
            accessibility_trusted(),
            "grant the terminal Accessibility first"
        );
        let env = |k: &str, d: u64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        let rounds = env("PASTE_ROUNDS", 60);
        let gap = Duration::from_millis(env("PASTE_GAP_MS", 40));
        let load = env("PROBE_LOAD", 0) as usize;

        let front_is_textedit = || {
            let asn = std::process::Command::new("lsappinfo")
                .arg("front")
                .output()
                .unwrap();
            let asn = String::from_utf8_lossy(&asn.stdout).trim().to_string();
            let info = std::process::Command::new("lsappinfo")
                .args(["info", "-only", "bundleid", &asn])
                .output()
                .unwrap();
            String::from_utf8_lossy(&info.stdout).contains("com.apple.TextEdit")
        };
        // Restores the clipboard (text only: an image on it is lost) and
        // stops the load threads even when a guard below panics. The two
        // TextEdit windows stay open for inspection; close them unsaved.
        struct Cleanup {
            clipboard: Option<String>,
            stop: Arc<AtomicBool>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::Relaxed);
                if let Some(text) = self.clipboard.take() {
                    let _ = copy_text(&text);
                }
            }
        }
        // A new file per run: rewriting one TextEdit still has open makes
        // it raise a "changed by another application" sheet that swallows
        // every paste.
        let run = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let stress_file = |variant: &str| {
            std::env::temp_dir().join(format!("mouthpiece-paste-stress-{run}-{variant}.txt"))
        };
        let stop = Arc::new(AtomicBool::new(false));
        let _cleanup = Cleanup {
            clipboard: arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.get_text().ok()),
            stop: stop.clone(),
        };
        let busy: Vec<_> = (0..load)
            .map(|_| {
                let stop = stop.clone();
                thread::spawn(move || {
                    let mut x = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    x
                })
            })
            .collect();

        let old_chord = |gap: Duration| {
            simulate(&EventType::KeyPress(Key::MetaLeft)).unwrap();
            thread::sleep(gap);
            simulate(&EventType::KeyPress(Key::KeyV)).unwrap();
            thread::sleep(Duration::from_millis(20));
            simulate(&EventType::KeyRelease(Key::KeyV)).unwrap();
            thread::sleep(Duration::from_millis(20));
            simulate(&EventType::KeyRelease(Key::MetaLeft)).unwrap();
        };

        let mut report = Vec::new();
        for variant in ["old", "new"] {
            let file = stress_file(variant);
            std::fs::write(&file, "").unwrap();
            std::process::Command::new("open")
                .args(["-a", "TextEdit"])
                .arg(&file)
                .status()
                .unwrap();
            let came_forward = (0..40).any(|_| {
                thread::sleep(Duration::from_millis(200));
                front_is_textedit()
            });
            assert!(came_forward, "TextEdit didn't come to the front");
            thread::sleep(Duration::from_millis(500));

            for i in 0..rounds {
                assert!(
                    front_is_textedit(),
                    "focus left TextEdit; stopped after {i} pastes"
                );
                let token = format!("[{i}]");
                copy_text(&token).unwrap();
                thread::sleep(Duration::from_millis(60));
                if variant == "old" {
                    old_chord(gap);
                } else {
                    command_chord_with_gap(Chord::Paste, gap.min(CHORD_GAP)).unwrap();
                }
                thread::sleep(Duration::from_millis(40));
            }

            thread::sleep(Duration::from_millis(300));
            assert!(
                front_is_textedit(),
                "focus left TextEdit before reading back"
            );
            copy_text("").unwrap();
            command_chord(Chord::SelectAll).unwrap();
            command_chord(Chord::Copy).unwrap();
            thread::sleep(Duration::from_millis(300));
            let text = arboard::Clipboard::new()
                .unwrap()
                .get_text()
                .unwrap_or_default();
            let landed = (0..rounds)
                .filter(|i| text.contains(&format!("[{i}]")))
                .count();
            let bare_v = text.matches('v').count();
            report.push(format!(
                "{variant}: {landed}/{rounds} pasted, {bare_v} bare v (gap {}ms, load {load}); read back {} chars: {:?}",
                gap.as_millis(),
                text.chars().count(),
                text.chars().take(60).collect::<String>()
            ));
        }

        stop.store(true, Ordering::Relaxed);
        for t in busy {
            let _ = t.join();
        }
        for line in &report {
            eprintln!("{line}");
        }
    }
}
