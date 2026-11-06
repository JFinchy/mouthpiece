//! Reaching an agent's exact terminal, not just its app's window.
//!
//! - Orca: the hook passes `$ORCA_TERMINAL_HANDLE`, and Orca's own CLI
//!   types the reply straight into that pane (`orca terminal send`). Focus
//!   never moves, so nothing has to be raised or put back.
//! - iTerm2 and Terminal: their AppleScript dictionaries list each tab's (or
//!   session's) terminal device, so the agent's own tab can be selected by
//!   tty before pasting. Needs the Automation permission, which macOS asks
//!   for the first time.
//!
//! Everything else falls back to picking a window by its title (agents.rs).

use std::process::Command;
use std::time::Duration;

/// Where Orca installs its CLI. Never taken from the link: a link only
/// names a terminal, it can't choose what Mouthpiece runs.
fn orca_cli() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    [
        Some(std::path::PathBuf::from(
            "/Applications/Orca.app/Contents/Resources/bin/orca",
        )),
        home.map(|h| h.join("Applications/Orca.app/Contents/Resources/bin/orca")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
}

/// Orca handles look like `term_<uuid>`; anything else is ignored.
pub fn valid_orca_handle(handle: &str) -> bool {
    handle.len() <= 80
        && handle.strip_prefix("term_").is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Text typed into a terminal without Enter must not contain line breaks
/// or control bytes, which would submit or edit the line.
pub fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split(' ')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<String, String> {
    use std::io::Read;
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                let mut out = String::new();
                let mut err = String::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut out);
                }
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut err);
                }
                return if status.success() {
                    Ok(out)
                } else {
                    let msg = err
                        .lines()
                        .chain(out.lines())
                        .find(|l| !l.trim().is_empty());
                    Err(msg
                        .unwrap_or("it failed")
                        .trim()
                        .chars()
                        .take(160)
                        .collect())
                };
            }
            None if started.elapsed() > timeout => {
                let _ = child.kill();
                return Err("it didn't answer in time".to_string());
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Types the reply into an Orca terminal, pressing Enter if asked.
pub fn orca_send(handle: &str, text: &str, press_enter: bool) -> Result<(), String> {
    if !valid_orca_handle(handle) {
        return Err("the agent's Orca terminal isn't known".to_string());
    }
    let cli = orca_cli().ok_or("Orca's command-line tool wasn't found")?;
    let mut cmd = Command::new(cli);
    cmd.args(["terminal", "send", "--terminal", handle]);
    let text = if press_enter {
        text.trim().to_string()
    } else {
        single_line(text)
    };
    if !text.is_empty() {
        cmd.args(["--text", &text]);
    }
    if press_enter {
        cmd.arg("--enter");
    }
    run_with_timeout(cmd, Duration::from_secs(5)).map(|_| ())
}

/// Shows the agent's Orca terminal (the widget pill's click).
pub fn orca_switch(handle: &str) -> Result<(), String> {
    if !valid_orca_handle(handle) {
        return Err("the agent's Orca terminal isn't known".to_string());
    }
    let cli = orca_cli().ok_or("Orca's command-line tool wasn't found")?;
    let mut cmd = Command::new(cli);
    cmd.args(["terminal", "switch", "--terminal", handle]);
    run_with_timeout(cmd, Duration::from_secs(5)).map(|_| ())
}

/// A tty as `ps` reports it ("ttys003"), checked so it can go into a script.
fn tty_device(tty: &str) -> Option<String> {
    let name = tty.trim_start_matches("/dev/");
    let ok = name.starts_with("tty")
        && name.len() <= 16
        && name.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| format!("/dev/{name}"))
}

/// AppleScript that selects the tab or session on `device` and brings its
/// window to the front. Prints "ok" when found.
pub fn select_tab_script(bundle_id: &str, device: &str) -> Option<String> {
    match bundle_id {
        "com.googlecode.iterm2" => Some(format!(
            r#"tell application id "com.googlecode.iterm2"
  repeat with w in windows
    repeat with t in tabs of w
      repeat with s in sessions of t
        if tty of s is "{device}" then
          select w
          select t
          select s
          activate
          return "ok"
        end if
      end repeat
    end repeat
  end repeat
end tell
return "missing""#
        )),
        "com.apple.Terminal" => Some(format!(
            r#"tell application id "com.apple.Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{device}" then
        set selected of t to true
        set index of w to 1
        activate
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "missing""#
        )),
        _ => None,
    }
}

/// Whether this app's tabs can be picked by tty.
pub fn can_select_by_tty(bundle_id: &str) -> bool {
    select_tab_script(bundle_id, "/dev/ttys000").is_some()
}

/// Selects the agent's own tab in iTerm2 or Terminal. `Ok(())` means that
/// tab is now the one showing in the front window.
pub fn select_tab(bundle_id: &str, tty: &str) -> Result<(), String> {
    let device = tty_device(tty).ok_or("its terminal device isn't known")?;
    let script = select_tab_script(bundle_id, &device).ok_or("tabs can't be picked in this app")?;
    let mut cmd = Command::new("/usr/bin/osascript");
    cmd.args(["-e", &script]);
    match run_with_timeout(cmd, Duration::from_secs(4)) {
        Ok(out) if out.trim() == "ok" => Ok(()),
        Ok(_) => Err("its tab is closed".to_string()),
        // -1743: the user hasn't allowed Mouthpiece to control the app.
        Err(err) if err.contains("-1743") => Err(
            "Mouthpiece isn't allowed to control it (System Settings > Privacy & Security > Automation)"
                .to_string(),
        ),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orca_handles_are_checked() {
        assert!(valid_orca_handle(
            "term_db0a3dca-a75a-4ed8-903d-79ad005922b6"
        ));
        assert!(!valid_orca_handle("term_"));
        assert!(!valid_orca_handle("term_abc; rm -rf"));
        assert!(!valid_orca_handle("/bin/sh"));
    }

    #[test]
    fn ttys_are_checked_before_going_into_a_script() {
        assert_eq!(tty_device("ttys003").as_deref(), Some("/dev/ttys003"));
        assert_eq!(tty_device("/dev/ttys012").as_deref(), Some("/dev/ttys012"));
        assert_eq!(tty_device("ttys0\" & do shell script \"x"), None);
        assert_eq!(tty_device("console"), None);
    }

    #[test]
    fn scripts_exist_for_iterm_and_terminal_only() {
        assert!(can_select_by_tty("com.googlecode.iterm2"));
        assert!(can_select_by_tty("com.apple.Terminal"));
        assert!(!can_select_by_tty("dev.warp.Warp-Stable"));
        let script = select_tab_script("com.apple.Terminal", "/dev/ttys004").unwrap();
        assert!(script.contains("\"/dev/ttys004\""));
    }

    #[test]
    fn unsubmitted_text_is_one_line() {
        assert_eq!(
            single_line("fix it\nand\tthen\x1b[A go"),
            "fix it and then [A go"
        );
    }
}
