//! Pointing at things while you talk to an agent. End a dictation with:
//!
//! - "attach the selection" (or ", with the selection"): the text selected
//!   in the app in front, as a code block.
//! - "attach the diff" / "attach my changes" (", with the diff"): `git diff
//!   HEAD` of the project you're in; "attach the staged changes" for
//!   `git diff --cached`.
//! - "look at this" (after a clause break, or on its own) / "attach a
//!   screenshot": a screenshot of the window you're looking at, saved to a
//!   temporary file whose path is added for the agent to open.
//!
//! These sit alongside "attach the error" (`error_context.rs`) and can be
//! combined: "why is this red, look at this, attach the diff". Captured
//! content is never logged, only sizes.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Selection,
    Diff,
    StagedDiff,
    Screenshot,
}

impl Kind {
    pub fn feature(self) -> &'static str {
        match self {
            Kind::Selection => "attach_selection",
            Kind::Diff | Kind::StagedDiff => "attach_diff",
            Kind::Screenshot => "attach_screenshot",
        }
    }
}

/// Phrases that are deliberate enough to match at the end on their own.
const TRIGGERS: &[(&str, Kind)] = &[
    ("attach the selected text", Kind::Selection),
    ("attach what i selected", Kind::Selection),
    ("attach the selection", Kind::Selection),
    ("attach my selection", Kind::Selection),
    ("attach selection", Kind::Selection),
    ("attach the staged changes", Kind::StagedDiff),
    ("attach the staged diff", Kind::StagedDiff),
    ("attach the diff", Kind::Diff),
    ("attach my diff", Kind::Diff),
    ("attach my changes", Kind::Diff),
    ("attach the changes", Kind::Diff),
    ("attach diff", Kind::Diff),
    ("attach a screenshot", Kind::Screenshot),
    ("attach the screenshot", Kind::Screenshot),
    ("attach screenshot", Kind::Screenshot),
];

/// Ordinary English too ("let me look at this", "I'm happy with the
/// diff"), so these count only after a clause break or as the whole
/// dictation.
const GUARDED: &[(&str, Kind)] = &[
    ("with the selection", Kind::Selection),
    ("with the diff", Kind::Diff),
    ("with my changes", Kind::Diff),
    ("take a look at this", Kind::Screenshot),
    ("look at this", Kind::Screenshot),
    ("see this", Kind::Screenshot),
];

/// Strips trailing attach phrases, last-spoken first, and returns what's
/// left plus the attachments in the order they were spoken.
pub fn extract(text: &str) -> (String, Vec<Kind>) {
    let mut rest = text.trim().to_string();
    let mut kinds = Vec::new();
    while let Some((prefix, kind)) = strip_one(&rest) {
        if !kinds.contains(&kind) {
            kinds.insert(0, kind);
        }
        rest = prefix;
        // "attach the selection and attach the diff": drop the "and" only
        // when another attach phrase comes before it.
        if let Some(before_and) = crate::clipboard::strip_suffix_ignore_case(&rest, " and") {
            if strip_one(before_and).is_some() {
                rest = before_and.to_string();
            }
        }
    }
    if kinds.is_empty() {
        (text.to_string(), kinds)
    } else {
        (rest, kinds)
    }
}

/// Removes one trailing attach phrase: what's left before it, and which.
fn strip_one(text: &str) -> Option<(String, Kind)> {
    let body = text.trim().trim_end_matches(['.', '!', '?']).trim_end();
    let candidates = TRIGGERS
        .iter()
        .map(|t| (t, false))
        .chain(GUARDED.iter().map(|t| (t, true)));
    for (&(phrase, kind), guarded) in candidates {
        let Some(prefix) = crate::clipboard::strip_suffix_ignore_case(body, phrase) else {
            continue;
        };
        if !prefix.is_empty() && !prefix.ends_with(' ') {
            continue;
        }
        let head = prefix.trim_end();
        if guarded && !head.is_empty() && !head.ends_with([',', '.', ';', '!', '?', ':']) {
            continue;
        }
        return Some((head.trim_end_matches([' ', ',', ';']).to_string(), kind));
    }
    None
}

/// Appends a fenced block with an info string ("diff", or "" for none).
pub fn fence(text: &str, block: &str, lang: &str) -> String {
    let longest_run = block
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let ticks = "`".repeat(longest_run.max(2) + 1);
    let fenced = format!("{ticks}{lang}\n{}\n{ticks}", block.trim_end_matches('\n'));
    if text.is_empty() {
        fenced
    } else {
        format!("{text}\n\n{fenced}")
    }
}

/// Longest diff attached, in lines; longer ones are cut with a note.
const MAX_DIFF_LINES: usize = 400;
const MAX_SELECTION_CHARS: usize = 20_000;

/// Keeps the first `max` lines, saying how many were left out.
pub fn cap_lines(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max {
        return text.trim_end().to_string();
    }
    format!(
        "{}\n... ({} more lines not shown)",
        lines[..max].join("\n"),
        lines.len() - max
    )
}

pub fn cap_selection(text: &str) -> String {
    if text.chars().count() <= MAX_SELECTION_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_SELECTION_CHARS).collect();
    format!("{cut}\n... (selection cut short)")
}

/// `git diff` in the project folder. `Ok(None)` when there are no changes.
pub fn read_diff(root: &Path, staged: bool) -> Result<Option<String>, String> {
    let mut cmd = Command::new("/usr/bin/git");
    cmd.arg("-C")
        .arg(root)
        .args(["--no-pager", "diff", "--no-color", "--no-ext-diff"]);
    if staged {
        cmd.arg("--cached");
    } else {
        cmd.arg("HEAD");
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("git couldn't read the changes".to_string());
    }
    let diff = String::from_utf8_lossy(&out.stdout).to_string();
    Ok((!diff.trim().is_empty()).then(|| cap_lines(&diff, MAX_DIFF_LINES)))
}

// ------------------------------------------------------------- screenshot

/// Picks the window to capture from on-screen windows, front to back:
/// `(window id, owner pid, layer)`. Mouthpiece's own windows never count.
/// When you're in a terminal or editor (talking to an agent there), the
/// window you mean is the one behind it, from another app; otherwise it's
/// the front window.
pub fn pick_window(
    windows: &[(u32, i32, i64)],
    own_pid: i32,
    front_pid: Option<i32>,
    front_is_agent_app: bool,
) -> Option<u32> {
    windows
        .iter()
        .filter(|(_, pid, layer)| *layer == 0 && *pid != own_pid)
        .find(|(_, pid, _)| !(front_is_agent_app && Some(*pid) == front_pid))
        .map(|(id, _, _)| *id)
}

#[cfg(target_os = "macos")]
fn on_screen_windows() -> Vec<(u32, i32, i64)> {
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerPID,
    };
    let Some(info) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    let info: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_get_rule(info.as_concrete_TypeRef()) };
    let number = |dict: &CFDictionary<CFString, CFType>, key| -> Option<i64> {
        let key = unsafe { CFString::wrap_under_get_rule(key) };
        dict.find(&key)?.downcast::<CFNumber>()?.to_i64()
    };
    info.iter()
        .filter_map(|dict| {
            Some((
                number(&dict, unsafe { kCGWindowNumber })? as u32,
                number(&dict, unsafe { kCGWindowOwnerPID })? as i32,
                number(&dict, unsafe { kCGWindowLayer }).unwrap_or(0),
            ))
        })
        .collect()
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

fn screenshot_dir() -> PathBuf {
    std::env::temp_dir().join("mouthpiece-screenshots")
}

/// Screenshots older than a day are removed, so they don't pile up.
fn clean_old_screenshots(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 24 * 3600);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Captures the window you're looking at to a PNG and returns its path.
#[cfg(target_os = "macos")]
pub fn capture_window(
    front_pid: Option<i32>,
    front_bundle: Option<&str>,
) -> Result<PathBuf, String> {
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        unsafe { CGRequestScreenCaptureAccess() };
        return Err("Allow Screen Recording for Mouthpiece to attach screenshots".to_string());
    }
    let agent_app = front_bundle.is_some_and(crate::project::is_terminal_or_editor);
    let id = pick_window(
        &on_screen_windows(),
        std::process::id() as i32,
        front_pid,
        agent_app,
    )
    .ok_or("No window to capture")?;
    let dir = screenshot_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    clean_old_screenshots(&dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("screenshot-{stamp}.png"));
    let status = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-o", &format!("-l{id}")])
        .arg(&path)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() || !path.exists() {
        return Err("The screenshot failed".to_string());
    }
    Ok(path)
}

#[cfg(not(target_os = "macos"))]
pub fn capture_window(_: Option<i32>, _: Option<&str>) -> Result<PathBuf, String> {
    Err("Screenshots need macOS".to_string())
}

/// How a screenshot is referred to: `@path` for terminal agents (Claude
/// Code and others read an image from an @-path), the plain path elsewhere.
pub fn screenshot_reference(text: &str, path: &Path, terminal: bool) -> String {
    let path = path.to_string_lossy();
    let reference = if terminal {
        format!("@{path}")
    } else {
        path.to_string()
    };
    if text.is_empty() {
        reference
    } else {
        format!("{text}\n\n{reference}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_triggers_are_stripped() {
        assert_eq!(
            extract("Explain this function, attach the selection."),
            ("Explain this function".into(), vec![Kind::Selection])
        );
        assert_eq!(
            extract("review it attach the diff"),
            ("review it".into(), vec![Kind::Diff])
        );
        assert_eq!(
            extract("Write a commit message, attach the staged changes"),
            ("Write a commit message".into(), vec![Kind::StagedDiff])
        );
        assert_eq!(
            extract("attach a screenshot"),
            (String::new(), vec![Kind::Screenshot])
        );
    }

    #[test]
    fn guarded_triggers_need_a_clause_break() {
        assert_eq!(
            extract("The header is misaligned, look at this"),
            ("The header is misaligned".into(), vec![Kind::Screenshot])
        );
        assert_eq!(
            extract("Look at this."),
            (String::new(), vec![Kind::Screenshot])
        );
        assert_eq!(
            extract("I'll look at this"),
            ("I'll look at this".into(), vec![])
        );
        assert_eq!(
            extract("I'm happy with the diff"),
            ("I'm happy with the diff".into(), vec![])
        );
        assert_eq!(
            extract("fix the test, with the diff"),
            ("fix the test".into(), vec![Kind::Diff])
        );
    }

    #[test]
    fn several_can_be_combined_in_spoken_order() {
        assert_eq!(
            extract("Why is this red, look at this, attach the diff"),
            ("Why is this red".into(), vec![Kind::Screenshot, Kind::Diff])
        );
        assert_eq!(
            extract("refactor it, attach the selection and attach the diff"),
            ("refactor it".into(), vec![Kind::Selection, Kind::Diff])
        );
    }

    #[test]
    fn words_ending_in_a_trigger_dont_count() {
        assert_eq!(extract("reattach diff"), ("reattach diff".into(), vec![]));
        assert_eq!(
            extract("nothing to attach"),
            ("nothing to attach".into(), vec![])
        );
    }

    #[test]
    fn fences_use_a_language_and_outlast_backticks() {
        assert_eq!(
            fence("see", "+a\n-b\n", "diff"),
            "see\n\n```diff\n+a\n-b\n```"
        );
        assert_eq!(fence("", "x ```` y", ""), "`````\nx ```` y\n`````");
    }

    #[test]
    fn long_diffs_are_cut_with_a_note() {
        let diff: String = (0..10).map(|i| format!("line {i}\n")).collect();
        assert_eq!(
            cap_lines(&diff, 3),
            "line 0\nline 1\nline 2\n... (7 more lines not shown)"
        );
        assert_eq!(cap_lines("a\nb\n", 3), "a\nb");
    }

    #[test]
    fn picks_the_window_behind_a_terminal() {
        // (id, pid, layer), front to back. 1 = Mouthpiece, 2 = terminal, 3 = browser.
        let windows = [(10, 1, 0), (11, 2, 0), (12, 3, 0), (13, 3, 25)];
        assert_eq!(pick_window(&windows, 1, Some(2), true), Some(12));
        assert_eq!(pick_window(&windows, 1, Some(2), false), Some(11));
        assert_eq!(pick_window(&[(13, 3, 25)], 1, None, false), None);
    }

    #[test]
    fn screenshots_are_at_paths_for_terminal_agents() {
        let p = Path::new("/tmp/mouthpiece-screenshots/s.png");
        assert_eq!(
            screenshot_reference("why", p, true),
            "why\n\n@/tmp/mouthpiece-screenshots/s.png"
        );
        assert_eq!(
            screenshot_reference("", p, false),
            "/tmp/mouthpiece-screenshots/s.png"
        );
    }

    #[test]
    fn diff_reads_a_real_repo() {
        let dir = std::env::temp_dir().join(format!("mp-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("/usr/bin/git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        git(&["add", "a.txt"]);
        assert!(read_diff(&dir, true).unwrap().unwrap().contains("+hello"));
        assert!(read_diff(&dir, false).unwrap().unwrap().contains("+hello"));
        git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "a",
        ]);
        assert_eq!(read_diff(&dir, false).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
