//! "Attach the error": end a dictation with the phrase and the most recent
//! error output is appended as a fenced code block, so a prompt to a coding
//! agent ("why does this fail, attach the error") doesn't need the stack
//! trace read aloud or copied by hand.
//!
//! Source order, decided in `recording.rs`: the frontmost terminal's visible
//! text (macOS Accessibility, see `read_terminal_text`), else the clipboard.
//! This file is the pure half (trigger detection, picking the last
//! error-looking block out of a screenful of text, fencing it) plus the AX
//! read. Captured content is never logged, only line counts.

/// Longest block attached, in lines. Longer ones keep their head and tail.
const MAX_LINES: usize = 60;
/// Lines past this are cut, so one minified bundle line can't flood a prompt.
const MAX_LINE_CHARS: usize = 400;
/// How far past the last error line the block runs (summary, "Time:" ...).
const TRAILING_LINES: usize = 8;
/// Two error lines further apart than this belong to different problems.
const CLUSTER_GAP: usize = 12;

/// Trailing phrases that are deliberate enough to match on their own.
const TRIGGERS: &[&str] = &[
    "attach the error message",
    "attach the last error",
    "attach the error",
    "attach error message",
    "attach error",
];
/// "...with the error" is also ordinary English ("I'm stuck with the
/// error"), so it counts only after a clause break or as the whole
/// utterance — the same rule as "send it" in `punctuation.rs`.
const GUARDED_TRIGGERS: &[&str] = &["with the error message", "with the error"];

/// Returns `(text_with_trigger_removed, attach_requested)`. Same shape as
/// `clipboard::try_extract_trigger`.
pub fn try_extract_trigger(text: &str) -> (String, bool) {
    let trimmed = text.trim();
    let without_trailing_punct = trimmed.trim_end_matches(['.', '!', '?']).trim_end();

    for phrase in TRIGGERS.iter().chain(GUARDED_TRIGGERS) {
        let guarded = GUARDED_TRIGGERS.contains(phrase);
        // Matched on the original text, not a lowercased copy: lowercasing
        // can change the byte length of earlier non-ASCII words.
        let Some(prefix) =
            crate::clipboard::strip_suffix_ignore_case(without_trailing_punct, phrase)
        else {
            continue;
        };
        if prefix.is_empty() {
            return (String::new(), true);
        }
        if !prefix.ends_with(' ') {
            continue;
        }
        if guarded && !prefix.trim_end().ends_with([',', '.', ';', '!', '?', ':']) {
            continue;
        }
        let remaining = prefix.trim_end_matches([' ', ',', ';']);
        return (remaining.to_string(), true);
    }
    (text.to_string(), false)
}

/// Appends `block` to `text` as a fenced code block.
pub fn attach(text: &str, block: &str) -> String {
    // A fence longer than any backtick run inside the block, so output that
    // itself contains ``` can't close it early.
    let longest_run = block
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest_run.max(2) + 1);
    let fenced = format!("{fence}\n{block}\n{fence}");
    if text.is_empty() {
        fenced
    } else {
        format!("{text}\n\n{fenced}")
    }
}

/// The last error-looking block in `text` (terminal scrollback or a copied
/// log), or `None` if nothing in it looks like an error.
///
/// "Last" means the final run of error lines, bounded by shell prompts: the
/// block starts at the first error line of that run (within the output of
/// the same command) and ends a few lines after the final error line, before
/// the next prompt.
pub fn extract_error_block(text: &str) -> Option<String> {
    let cleaned = strip_ansi(text);
    let lines: Vec<&str> = cleaned.lines().map(str::trim_end).collect();

    let last_error = lines.iter().rposition(|l| is_error_line(l))?;

    // Lower bound: the command that printed this error started after the
    // previous prompt line.
    let floor = lines[..last_error]
        .iter()
        .rposition(|l| is_prompt_line(l))
        .map_or(0, |p| p + 1);

    // Walk back through error lines that belong to the same cluster.
    let mut start = last_error;
    let mut previous_error = last_error;
    for i in (floor..last_error).rev() {
        if is_error_line(lines[i]) {
            if previous_error - i > CLUSTER_GAP {
                break;
            }
            start = i;
            previous_error = i;
        } else if previous_error - i > CLUSTER_GAP {
            break;
        }
    }

    // Forward: whatever the failing tool printed after its last error line
    // (summaries, hints), up to the next prompt.
    let mut end = last_error;
    for (offset, line) in lines[last_error + 1..].iter().enumerate() {
        if offset >= TRAILING_LINES || is_prompt_line(line) {
            break;
        }
        end = last_error + 1 + offset;
    }

    let block: Vec<&str> = lines[start..=end].to_vec();
    Some(cap_lines(&block))
}

/// The whole of `text` (trimmed, capped) if it contains an error line: for a
/// clipboard the user copied on purpose, everything they copied is wanted,
/// not just the tail.
pub fn clipboard_error_text(text: &str) -> Option<String> {
    let cleaned = strip_ansi(text);
    if !cleaned.lines().any(is_error_line) {
        return None;
    }
    let lines: Vec<&str> = cleaned.trim().lines().map(str::trim_end).collect();
    Some(cap_lines(&lines))
}

/// Keeps the first and last lines of an over-long block, noting the gap.
fn cap_lines(lines: &[&str]) -> String {
    let clip = |l: &str| -> String {
        if l.chars().count() > MAX_LINE_CHARS {
            let cut: String = l.chars().take(MAX_LINE_CHARS).collect();
            format!("{cut} [...]")
        } else {
            l.to_string()
        }
    };
    let mut kept: Vec<String> = if lines.len() <= MAX_LINES {
        lines.iter().map(|l| clip(l)).collect()
    } else {
        let head = 20;
        let tail = MAX_LINES - head - 1;
        let omitted = lines.len() - head - tail;
        let mut v: Vec<String> = lines[..head].iter().map(|l| clip(l)).collect();
        v.push(format!("[... {omitted} lines omitted ...]"));
        v.extend(lines[lines.len() - tail..].iter().map(|l| clip(l)));
        v
    };
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    kept.join("\n")
}

/// Does this single line look like part of an error report? Deliberately
/// anchored on well-known shapes (rustc/cargo, tsc, Python, Node, Jest,
/// Vitest, Playwright, pytest, git) instead of the bare word "error", which
/// shows up in prose, file names and `errors: 0` summaries.
fn is_error_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_lowercase();

    const STARTS: &[&str] = &[
        "error:",
        "error[",
        "error ts",
        "fatal:",
        "panic:",
        "npm err!",
        "traceback (most recent call last)",
        "caused by:",
        "uncaught ",
        "unhandled ",
        "exception in thread",
        "during handling of the above exception",
        "the above exception was the direct cause",
        "for more information about this error",
        "some errors have detailed explanation",
        "failures:",
        "test result: failed",
        "fail ",
        "failed ",
        "● ",
        "✕ ",
        "✗ ",
        "✘ ",
        "× ",
    ];
    if STARTS.iter().any(|s| lower.starts_with(s)) || lower == "fail" {
        return true;
    }
    if lower.contains("panicked at")
        || lower.contains(": error ts")
        || lower.contains(" - error ts")
        || lower.ends_with("... failed")
        || lower.contains("short test summary info")
        || (lower.starts_with('=') && lower.contains(" failures "))
    {
        return true;
    }
    // pytest assertion lines: "E       assert 1 == 2".
    if line.starts_with("E   ") {
        return true;
    }
    // Playwright failure headers: "1) [chromium] › tests/a.spec.ts:10:5 › x".
    if let Some((num, rest)) = t.split_once(") ") {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) && rest.starts_with('[') {
            return true;
        }
    }
    has_failure_count(&lower) || is_exception_line(t)
}

/// "3 failed", "Tests: 1 failed, 4 passed", but not "0 failed".
fn has_failure_count(lower: &str) -> bool {
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(2)
        .any(|w| w[1] == "failed" && w[0].parse::<u32>().is_ok_and(|n| n > 0))
}

/// "TypeError: x is not a function", "java.lang.NullPointerException: ...",
/// "AssertionError" — an identifier ending in Error/Exception, then ':' or
/// end of line.
fn is_exception_line(t: &str) -> bool {
    let end = t
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(t.len());
    let name = &t[..end];
    let rest = t[end..].trim_start();
    (name.ends_with("Error") || name.ends_with("Exception"))
        && name.chars().next().is_some_and(char::is_alphabetic)
        && (rest.is_empty() || rest.starts_with(':'))
}

/// A shell prompt line, which ends the output of the previous command.
fn is_prompt_line(line: &str) -> bool {
    let t = line.trim();
    // (A bare leading "❯" is not a prompt: Vitest prints it before file names.)
    if t.starts_with("$ ") || t.starts_with("% ") || t == "❯" || t == "➜" {
        return true;
    }
    if t.contains(" ❯ ") || t.contains(" ➜ ") {
        return true;
    }
    // "user@host dir %", "~/proj main ❯": a short line ending in a prompt
    // character (and not a sentence of output).
    t.chars().count() <= 100
        && t.ends_with(['$', '%', '❯', '➜', '#'])
        && !t.starts_with(['#', '/', '*', '-'])
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// Where the visible text of the frontmost terminal came from, for tests
/// and logs.
#[cfg(target_os = "macos")]
pub use ax::{read_selected_text, read_terminal_text};

#[cfg(not(target_os = "macos"))]
pub fn read_terminal_text(_pid: i32) -> Option<String> {
    None
}

/// Bundle ids of terminal emulators worth reading with Accessibility.
pub fn is_terminal_app(bundle_id: &str) -> bool {
    crate::project::is_terminal(bundle_id)
}

/// Raw Accessibility (AX) reads. Terminal.app and iTerm2 expose their
/// buffer as the AXValue of a text area; GPU terminals and Electron ones
/// (Warp, kitty, Alacritty, Hyper, VS Code's terminal) mostly don't, and
/// return `None` so the caller falls back to the clipboard.
#[cfg(target_os = "macos")]
mod ax {
    use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
    use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
    use std::ffi::c_void;

    type AXUIElementRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
    }

    /// Owned CF reference, released on drop.
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }

    fn copy_attr(element: AXUIElementRef, name: &str) -> Option<Owned> {
        let attr = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
        };
        (status == 0 && !value.is_null()).then(|| Owned(value))
    }

    fn as_string(value: &Owned) -> Option<String> {
        unsafe {
            if CFGetTypeID(value.0) != CFStringGetTypeID() {
                return None;
            }
            Some(CFString::wrap_under_get_rule(value.0 as CFStringRef).to_string())
        }
    }

    fn children(element: AXUIElementRef) -> Vec<Owned> {
        let Some(array) = copy_attr(element, "AXChildren") else {
            return Vec::new();
        };
        unsafe {
            if CFGetTypeID(array.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            let count = CFArrayGetCount(array.0 as _);
            (0..count)
                .filter_map(|i| {
                    let child = CFArrayGetValueAtIndex(array.0 as _, i);
                    // The array owns `child`; retain so it outlives the array.
                    (!child.is_null()).then(|| {
                        core_foundation::base::CFRetain(child);
                        Owned(child)
                    })
                })
                .collect()
        }
    }

    /// Selected text, else the full value, of the text-bearing element in
    /// the app's focused window.
    pub fn read_terminal_text(pid: i32) -> Option<String> {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }
            let app = Owned(app);
            AXUIElementSetMessagingTimeout(app.0, 1.0);

            // Fast path: the focused element is the text area.
            if let Some(focused) = copy_attr(app.0, "AXFocusedUIElement") {
                if let Some(text) = text_of(focused.0) {
                    return Some(text);
                }
            }

            // Otherwise search the focused window's tree for the largest
            // text value (bounded, so an odd app can't stall dictation).
            let window = copy_attr(app.0, "AXFocusedWindow")?;
            let mut queue = vec![(window, 0usize)];
            let mut visited = 0;
            let mut best: Option<String> = None;
            while let Some((node, depth)) = (!queue.is_empty()).then(|| queue.remove(0)) {
                visited += 1;
                if visited > 300 {
                    break;
                }
                if let Some(role) = copy_attr(node.0, "AXRole").and_then(|r| as_string(&r)) {
                    if role == "AXTextArea" {
                        if let Some(text) = text_of(node.0) {
                            if best.as_ref().is_none_or(|b| text.len() > b.len()) {
                                best = Some(text);
                            }
                        }
                    }
                }
                if depth < 8 {
                    for child in children(node.0) {
                        queue.push((child, depth + 1));
                    }
                }
            }
            best
        }
    }

    /// The text selected in the app's focused element, if any. Works in
    /// most native and Electron text views; never reads anything else.
    pub fn read_selected_text(pid: i32) -> Option<String> {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }
            let app = Owned(app);
            AXUIElementSetMessagingTimeout(app.0, 1.0);
            let focused = copy_attr(app.0, "AXFocusedUIElement")?;
            copy_attr(focused.0, "AXSelectedText")
                .and_then(|v| as_string(&v))
                .filter(|s| !s.trim().is_empty())
        }
    }

    fn text_of(element: AXUIElementRef) -> Option<String> {
        // A selection is an explicit choice of what to attach.
        if let Some(sel) = copy_attr(element, "AXSelectedText").and_then(|v| as_string(&v)) {
            if !sel.trim().is_empty() {
                return Some(sel);
            }
        }
        copy_attr(element, "AXValue")
            .and_then(|v| as_string(&v))
            .filter(|s| !s.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_after_text_whose_lowercase_changes_length() {
        // "İ" lowercases to two chars, which used to misplace the cut.
        assert_eq!(
            try_extract_trigger("İzmir build failed, attach the error"),
            ("İzmir build failed".to_string(), true)
        );
        assert_eq!(
            try_extract_trigger("Attach the error."),
            (String::new(), true)
        );
    }

    fn block(text: &str) -> String {
        extract_error_block(text).expect("expected an error block")
    }

    const RUST_PANIC: &str = "\
$ cargo test
   Compiling tauri-app v0.2.0 (/Users/jake/dev-whisper/src-tauri)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.21s
     Running unittests src/lib.rs

running 2 tests
test pipeline::tests::ok ... ok
test history::tests::load ... FAILED

failures:

---- history::tests::load stdout ----

thread 'history::tests::load' panicked at src/history.rs:212:9:
assertion `left == right` failed
  left: 3
 right: 4
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    history::tests::load

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out

error: test failed, to rerun pass `--lib`
$ ";

    #[test]
    fn rust_panic_run_is_captured_from_its_first_error_line() {
        let b = block(RUST_PANIC);
        assert!(b.starts_with("test history::tests::load ... FAILED"), "{b}");
        assert!(b.contains("panicked at src/history.rs:212:9"));
        assert!(b.contains("assertion `left == right` failed"));
        assert!(b.ends_with("error: test failed, to rerun pass `--lib`"));
        assert!(!b.contains("Compiling"));
        assert!(!b.contains("$ "));
    }

    #[test]
    fn cargo_compile_error() {
        let log = "\
$ cargo build
   Compiling tauri-app v0.2.0 (/Users/jake/dev-whisper/src-tauri)
warning: unused variable: `x`
 --> src/lib.rs:3:9
  |
3 |     let x = 1;
  |         ^ help: if this is intentional, prefix it with an underscore: `_x`

error[E0308]: mismatched types
  --> src/pipeline.rs:41:20
   |
41 |     let n: u32 = \"a\";
   |            ---   ^^^ expected `u32`, found `&str`
   |            |
   |            expected due to this

error: could not compile `tauri-app` (lib) due to 1 previous error
jake@mac dev-whisper %";
        let b = block(log);
        assert!(b.starts_with("error[E0308]: mismatched types"), "{b}");
        assert!(b.contains("expected `u32`, found `&str`"));
        assert!(b.ends_with("due to 1 previous error"));
        assert!(!b.contains("unused variable"));
    }

    #[test]
    fn tsc_errors() {
        let log = "\
~/proj main ❯ bunx tsc --noEmit
src/App.tsx(12,7): error TS2322: Type 'string' is not assignable to type 'number'.
src/lib/util.ts(40,3): error TS2304: Cannot find name 'foo'.

Found 2 errors in 2 files.

Errors  Files
     1  src/App.tsx:12
     1  src/lib/util.ts:40
~/proj main ❯ ";
        let b = block(log);
        assert!(b.starts_with("src/App.tsx(12,7): error TS2322"), "{b}");
        assert!(b.contains("Found 2 errors in 2 files."));
        assert!(b.contains("src/lib/util.ts:40"));
        assert!(!b.contains("bunx tsc"));
    }

    #[test]
    fn python_traceback() {
        let log = "\
$ python manage.py migrate
Operations to perform:
  Apply all migrations: app
Traceback (most recent call last):
  File \"manage.py\", line 22, in <module>
    main()
  File \"/usr/lib/python3/site-packages/django/db/backends/utils.py\", line 89, in _execute
    return self.cursor.execute(sql, params)
django.db.utils.OperationalError: no such table: app_user
$ ";
        let b = block(log);
        assert!(b.starts_with("Traceback (most recent call last):"), "{b}");
        assert!(b.contains("File \"manage.py\", line 22"));
        assert!(b.ends_with("OperationalError: no such table: app_user"));
        assert!(!b.contains("Operations to perform"));
    }

    #[test]
    fn jest_failure() {
        let log = "\
% npx jest
 FAIL  src/sum.test.ts
  ● adds numbers

    expect(received).toBe(expected) // Object.is equality

    Expected: 4
    Received: 3

      5 |   expect(sum(1, 2)).toBe(4);
        |                     ^
      at Object.<anonymous> (src/sum.test.ts:5:23)

Test Suites: 1 failed, 1 total
Tests:       1 failed, 1 total
Snapshots:   0 total
Time:        1.2 s
Ran all test suites.
% ";
        let b = block(log);
        assert!(b.starts_with(" FAIL  src/sum.test.ts"), "{b}");
        assert!(b.contains("Expected: 4"));
        assert!(b.contains("Tests:       1 failed, 1 total"));
        assert!(b.ends_with("Ran all test suites."));
    }

    #[test]
    fn vitest_failure() {
        let log = "\
$ bun run test
 ❯ src/sum.test.ts (1 test | 1 failed) 4ms
   × adds numbers 3ms
     → expected 3 to be 4 // Object.is equality

⎯⎯⎯⎯⎯⎯ Failed Tests 1 ⎯⎯⎯⎯⎯⎯

 FAIL  src/sum.test.ts > adds numbers
AssertionError: expected 3 to be 4 // Object.is equality

- Expected
+ Received

- 4
+ 3

 ❯ src/sum.test.ts:6:19

 Test Files  1 failed (1)
      Tests  1 failed (1)
   Duration  312ms
$ ";
        let b = block(log);
        assert!(b.contains("AssertionError: expected 3 to be 4"), "{b}");
        assert!(b.contains("Test Files  1 failed (1)"));
        assert!(b.contains("Duration  312ms"));
        assert!(!b.contains("bun run test"));
    }

    #[test]
    fn playwright_failure() {
        let log = "\
$ bunx playwright test
Running 2 tests using 1 worker

  1) [webkit] › e2e/widget.spec.ts:12:3 › shows the pasted note ──────────────

    Error: expect(locator).toHaveText(expected)

    Expected string: \"Pasted\"
    Received string: \"\"
    Timeout: 5000ms

      14 |     await expect(page.locator(\".status\")).toHaveText(\"Pasted\");
         |                                           ^
        at /Users/jake/dev-whisper/e2e/widget.spec.ts:14:43

  1 failed
    [webkit] › e2e/widget.spec.ts:12:3 › shows the pasted note ───────────────
  1 passed (6.1s)
$ ";
        let b = block(log);
        assert!(b.starts_with("  1) [webkit]"), "{b}");
        assert!(b.contains("Expected string: \"Pasted\""));
        assert!(b.contains("1 passed (6.1s)"));
        assert!(!b.contains("Running 2 tests"));
    }

    #[test]
    fn node_stack_trace() {
        let log = "\
$ node server.js
Listening on 3000
/app/server.js:14
    throw new TypeError('x is not a function');
    ^

TypeError: x is not a function
    at handler (/app/server.js:14:11)
    at Layer.handle [as handle_request] (/app/node_modules/express/lib/router/layer.js:95:5)

Node.js v20.11.0
$ ";
        let b = block(log);
        assert!(b.starts_with("TypeError: x is not a function"), "{b}");
        assert!(b.contains("at handler (/app/server.js:14:11)"));
    }

    #[test]
    fn only_the_latest_command_is_attached() {
        let log = "\
$ cargo build
error: could not compile `old` due to 1 previous error
$ cargo build
   Compiling new v0.1.0
error[E0432]: unresolved import `foo`
 --> src/main.rs:1:5
$ ";
        let b = block(log);
        assert!(b.starts_with("error[E0432]"), "{b}");
        assert!(!b.contains("`old`"));
    }

    #[test]
    fn clean_output_has_no_error_block() {
        let log = "\
$ cargo test
running 3 tests
test a ... ok
test b ... ok
test c ... ok

test result: ok. 3 passed; 0 failed; 0 ignored
Tests:       0 failed, 3 passed
I should handle errors more gracefully in the retry logic.
$ ";
        assert_eq!(extract_error_block(log), None);
        assert_eq!(extract_error_block(""), None);
        assert_eq!(clipboard_error_text("https://example.com/docs"), None);
    }

    #[test]
    fn long_blocks_keep_head_and_tail_within_the_cap() {
        let mut log = String::from("Traceback (most recent call last):\n");
        for i in 0..200 {
            log.push_str(&format!("  File \"m{i}.py\", line {i}, in f\n"));
        }
        log.push_str("RuntimeError: boom\n");
        let b = clipboard_error_text(&log).unwrap();
        assert!(b.lines().count() <= MAX_LINES);
        assert!(b.starts_with("Traceback"));
        assert!(b.contains("lines omitted"));
        assert!(b.ends_with("RuntimeError: boom"));
    }

    #[test]
    fn very_long_lines_are_clipped() {
        let log = format!("error: {}\n", "x".repeat(5000));
        let b = block(&log);
        assert!(b.chars().count() < MAX_LINE_CHARS + 20);
    }

    #[test]
    fn ansi_colors_are_stripped_before_matching() {
        let log = "\u{1b}[1m\u{1b}[31merror\u{1b}[0m: build failed\r\n";
        assert_eq!(block(log), "error: build failed");
    }

    #[test]
    fn clipboard_text_is_attached_whole() {
        let clip = "Something went wrong\nTypeError: x is undefined\n    at a (b.js:1:1)\n";
        assert_eq!(
            clipboard_error_text(clip).unwrap(),
            "Something went wrong\nTypeError: x is undefined\n    at a (b.js:1:1)"
        );
    }

    #[test]
    fn trigger_phrases() {
        assert_eq!(
            try_extract_trigger("Why does this fail attach the error"),
            ("Why does this fail".to_string(), true)
        );
        assert_eq!(
            try_extract_trigger("Why does this fail, attach the error message."),
            ("Why does this fail".to_string(), true)
        );
        assert_eq!(
            try_extract_trigger("Fix this. With the error."),
            ("Fix this.".to_string(), true)
        );
        assert_eq!(
            try_extract_trigger("Attach the error"),
            (String::new(), true)
        );
        assert_eq!(try_extract_trigger("with the error"), (String::new(), true));
    }

    #[test]
    fn with_the_error_in_a_sentence_is_ordinary_text() {
        for text in [
            "I'm stuck with the error",
            "Please deal with the error",
            "Attach the error handler to the router",
            "reattach the error",
            "This is the problem with the error message",
        ] {
            assert_eq!(
                try_extract_trigger(text),
                (text.to_string(), false),
                "{text}"
            );
        }
    }

    #[test]
    fn attach_fences_the_block() {
        assert_eq!(
            attach("Why does this fail?", "error: boom"),
            "Why does this fail?\n\n```\nerror: boom\n```"
        );
        assert_eq!(attach("", "error: boom"), "```\nerror: boom\n```");
    }

    #[test]
    fn attach_uses_a_longer_fence_when_the_block_has_backticks() {
        let out = attach("Look", "error: bad ```fence``` here");
        assert!(out.starts_with("Look\n\n````\n"));
        assert!(out.ends_with("\n````"));
    }
}
