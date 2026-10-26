//! "Scratch that" / "undo that" as a whole dictation: take back the last
//! Mouthpiece paste.
//!
//! Mid-sentence, "scratch that" stays a backtrack correction
//! (`backtrack.rs`: "meet at two scratch that three"). This is the other
//! case, where it is the entire utterance, which backtrack leaves alone
//! because nothing follows the trigger.
//!
//! What is safe depends on the app, and the rule throughout is that we
//! never remove more than we pasted:
//!
//! - Editors, browsers, chat and notes apps: send Cmd+Z. A paste is one
//!   undo step there. Only within [`EDITOR_WINDOW`], since typing after the
//!   paste would make Cmd+Z undo that typing instead.
//! - Terminals (and anything else that doesn't undo): send exactly N
//!   backspaces, N being the pasted length, but only for a short, single-line,
//!   ASCII paste that was not submitted, within [`TERMINAL_WINDOW`]. Ctrl+U
//!   would clear the whole line including what the user typed before, so it
//!   is never used. The count can still be wrong if the user typed after
//!   the paste, hence the short window.
//! - Otherwise (a different app is frontmost, a multi-line paste, the line
//!   was submitted with "press enter", too much time passed): do nothing
//!   and say so.
//!
//! The record keeps only a length, never the pasted text.

use std::time::{Duration, Instant};

pub const EDITOR_WINDOW: Duration = Duration::from_secs(30);
pub const TERMINAL_WINDOW: Duration = Duration::from_secs(15);
/// Longer pastes may be collapsed by the target ("[Pasted text +12 lines]"
/// in agent TUIs), which would make a character count wrong.
const MAX_BACKSPACE_CHARS: usize = 300;

/// Whether the whole utterance is an undo command ("Scratch that.").
pub fn is_undo_command(text: &str) -> bool {
    let lower = text
        .trim()
        .trim_end_matches(['.', '!', '?', ','])
        .trim()
        .to_lowercase();
    matches!(lower.as_str(), "scratch that" | "undo that")
}

/// What the last paste looked like, captured when it happened.
#[derive(Debug, Clone, PartialEq)]
pub struct LastPaste {
    pub bundle_id: Option<String>,
    pub chars: usize,
    pub multiline: bool,
    pub ascii: bool,
    /// An Enter followed the paste ("press enter" / "send it"): the line is
    /// gone from the input, and a backspace would eat something else.
    pub submitted: bool,
    pub at: Instant,
}

impl LastPaste {
    pub fn new(text: &str, bundle_id: Option<String>, submitted: bool, at: Instant) -> Self {
        Self {
            bundle_id,
            chars: text.chars().count(),
            multiline: text.contains('\n'),
            ascii: text.is_ascii(),
            submitted,
            at,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    CmdZ,
    Backspaces(usize),
    /// Nothing is sent. The string is the widget note.
    Refuse(&'static str),
}

pub const UNSAFE_NOTE: &str = "Couldn't undo safely here";
pub const NOTHING_NOTE: &str = "Nothing to undo";

pub fn plan(last: Option<&LastPaste>, frontmost: Option<&str>, now: Instant) -> Plan {
    let Some(last) = last else {
        return Plan::Refuse(NOTHING_NOTE);
    };
    let Some(bundle) = last.bundle_id.as_deref() else {
        return Plan::Refuse(UNSAFE_NOTE);
    };
    if frontmost != Some(bundle) || last.submitted || last.chars == 0 {
        return Plan::Refuse(UNSAFE_NOTE);
    }
    let age = now.saturating_duration_since(last.at);

    if crate::error_context::is_terminal_app(bundle) {
        let safe = !last.multiline
            && last.ascii
            && last.chars <= MAX_BACKSPACE_CHARS
            && age <= TERMINAL_WINDOW;
        return if safe {
            Plan::Backspaces(last.chars)
        } else {
            Plan::Refuse(UNSAFE_NOTE)
        };
    }
    if age <= EDITOR_WINDOW {
        Plan::CmdZ
    } else {
        Plan::Refuse(UNSAFE_NOTE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERM: &str = "com.apple.Terminal";
    const NOTES: &str = "com.apple.Notes";

    fn paste(text: &str, app: &str, submitted: bool, at: Instant) -> LastPaste {
        LastPaste::new(text, Some(app.to_string()), submitted, at)
    }

    #[test]
    fn only_the_whole_utterance_is_an_undo_command() {
        for yes in [
            "Scratch that.",
            "scratch that",
            "Undo that!",
            " undo that. ",
        ] {
            assert!(is_undo_command(yes), "{yes}");
        }
        for no in [
            "call at three scratch that four",
            "wait, scratch that and redo it",
            "I need to undo that change in the diff",
            "scratch that function",
            "",
        ] {
            assert!(!is_undo_command(no), "{no}");
        }
    }

    #[test]
    fn backtrack_still_owns_mid_sentence_scratch_that() {
        let corrected =
            crate::backtrack::try_backtrack("call the client at three scratch that four");
        assert_eq!(corrected, "call the client at four");
        assert!(!is_undo_command(
            "call the client at three scratch that four"
        ));
    }

    #[test]
    fn editor_gets_cmd_z_within_the_window() {
        let t0 = Instant::now();
        let last = paste("Hello there, world.\nSecond line", NOTES, false, t0);
        assert_eq!(
            plan(Some(&last), Some(NOTES), t0 + Duration::from_secs(5)),
            Plan::CmdZ
        );
        assert_eq!(
            plan(Some(&last), Some(NOTES), t0 + Duration::from_secs(31)),
            Plan::Refuse(UNSAFE_NOTE)
        );
    }

    #[test]
    fn terminal_gets_exactly_the_pasted_length_in_backspaces() {
        let t0 = Instant::now();
        let last = paste("fix the flaky test", TERM, false, t0);
        assert_eq!(
            plan(Some(&last), Some(TERM), t0 + Duration::from_secs(4)),
            Plan::Backspaces(18)
        );
    }

    #[test]
    fn terminal_refuses_anything_it_cannot_count() {
        let t0 = Instant::now();
        let soon = t0 + Duration::from_secs(3);
        let cases = [
            paste("two\nlines", TERM, false, t0),
            paste("café au lait", TERM, false, t0),
            paste(&"x".repeat(301), TERM, false, t0),
            paste("send this", TERM, true, t0),
        ];
        for last in cases {
            assert_eq!(
                plan(Some(&last), Some(TERM), soon),
                Plan::Refuse(UNSAFE_NOTE)
            );
        }
        let old = paste("fix it", TERM, false, t0);
        assert_eq!(
            plan(Some(&old), Some(TERM), t0 + Duration::from_secs(16)),
            Plan::Refuse(UNSAFE_NOTE)
        );
    }

    #[test]
    fn refuses_when_the_app_changed_or_nothing_was_pasted() {
        let t0 = Instant::now();
        let last = paste("hello", NOTES, false, t0);
        assert_eq!(plan(Some(&last), Some(TERM), t0), Plan::Refuse(UNSAFE_NOTE));
        assert_eq!(plan(Some(&last), None, t0), Plan::Refuse(UNSAFE_NOTE));
        assert_eq!(plan(None, Some(NOTES), t0), Plan::Refuse(NOTHING_NOTE));
        let unknown = LastPaste::new("hello", None, false, t0);
        assert_eq!(
            plan(Some(&unknown), Some(NOTES), t0),
            Plan::Refuse(UNSAFE_NOTE)
        );
    }
}
