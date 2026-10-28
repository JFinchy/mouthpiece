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

    // The Meta keydown needs to actually register as "held" with the OS
    // before the V keydown arrives, or the frontmost app sees a bare "v"
    // instead of Cmd+V — observed intermittently with `send`'s normal 20ms
    // gap. A longer settle specifically here (not on every event) targets
    // that race without slowing down the rest of the sequence.
    send(EventType::KeyPress(Key::MetaLeft))?;
    thread::sleep(Duration::from_millis(40));
    send(EventType::KeyPress(Key::KeyV))?;
    send(EventType::KeyRelease(Key::KeyV))?;
    send(EventType::KeyRelease(Key::MetaLeft))?;
    Ok(())
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
    send(EventType::KeyPress(Key::MetaLeft))?;
    thread::sleep(Duration::from_millis(40));
    send(EventType::KeyPress(Key::KeyZ))?;
    send(EventType::KeyRelease(Key::KeyZ))?;
    send(EventType::KeyRelease(Key::MetaLeft))?;
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
}
