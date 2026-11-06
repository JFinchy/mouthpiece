//! Whisper writes a pause to think as punctuation: "for the... Amazon
//! Prime", "when the-- computer turns on", "the — or does". Almost always
//! that's hesitation, not meaning, so the marks are removed here, before the
//! spoken punctuation commands run. Saying "ellipsis" or "em dash" still
//! inserts one on purpose (see `punctuation.rs`).
//!
//! What replaces a mark depends on the words around it:
//! - after a word like "the", "for" or "to" the sentence can't have ended,
//!   so the words are simply joined ("for the Amazon Prime"), and a common
//!   word Whisper capitalized after the pause is lowercased again;
//! - otherwise a capitalized next word means Whisper heard a new sentence:
//!   "for learning... Do we have" becomes "for learning. Do we have";
//! - at the end, a trailing-off "..." becomes a period.
//!
//! Dashes count only when followed by a space or the end, so command-line
//! flags (`--amend`) and hyphenated words are never touched.

const MARKS: &[&str] = &["...", "…", "--", "—", "–", "-"];

/// Words after which a sentence can't end: articles, prepositions,
/// conjunctions, auxiliaries, possessives.
const CONTINUING: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "my", "your", "our", "their", "his", "her",
    "its", "to", "for", "of", "in", "on", "at", "by", "with", "from", "into", "about", "like",
    "and", "or", "but", "so", "if", "because", "than", "as", "is", "are", "was", "were", "be",
    "been", "am", "do", "does", "did", "have", "has", "had", "can", "could", "will", "would",
    "should", "might", "must", "not", "some", "any", "all", "every", "each", "which", "what",
    "when", "where", "how", "who", "why", "just", "really", "very", "also", "then", "it's",
    "there's", "we", "i", "you", "they", "he", "she", "it", "we're", "you're", "they're", "i'm",
    "let's", "maybe", "um", "uh",
];

/// Everyday words that are never proper nouns, so they can safely be
/// lowercased when Whisper capitalized them after a mid-sentence pause.
const COMMON: &[&str] = &[
    "a",
    "an",
    "the",
    "this",
    "that",
    "these",
    "those",
    "my",
    "your",
    "our",
    "their",
    "his",
    "her",
    "its",
    "to",
    "for",
    "of",
    "in",
    "on",
    "at",
    "by",
    "with",
    "from",
    "into",
    "about",
    "and",
    "or",
    "but",
    "so",
    "if",
    "because",
    "then",
    "is",
    "are",
    "was",
    "were",
    "be",
    "do",
    "does",
    "did",
    "have",
    "has",
    "had",
    "can",
    "could",
    "will",
    "would",
    "should",
    "not",
    "all",
    "some",
    "any",
    "every",
    "each",
    "what",
    "when",
    "where",
    "how",
    "who",
    "why",
    "which",
    "we",
    "you",
    "they",
    "he",
    "she",
    "it",
    "there",
    "here",
    "just",
    "maybe",
    "like",
    "make",
    "get",
    "go",
    "see",
    "use",
    "add",
    "set",
    "put",
    "take",
    "give",
    "try",
    "fix",
    "think",
    "want",
    "need",
    "know",
    "say",
    "look",
    "start",
    "stop",
    "run",
    "show",
    "turn",
    "one",
    "two",
    "three",
    "thing",
    "things",
    "way",
    "time",
    "something",
    "everything",
    "nothing",
    "also",
    "even",
    "still",
    "only",
    "really",
    "very",
    "more",
    "most",
    "other",
    "new",
    "same",
    "good",
    "right",
    "now",
    "again",
    "out",
    "up",
    "down",
    "over",
    "back",
    "boot",
    "computer",
    "app",
    "file",
    "code",
    "test",
    "tests",
    "setting",
    "settings",
];

/// `text` with Whisper's pause marks resolved, line by line (a list from
/// `expand_lists` keeps its line breaks).
pub fn remove_pause_marks(text: &str) -> String {
    text.split('\n')
        .map(remove_in_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn remove_in_line(line: &str) -> String {
    let mut out = line.to_string();
    // Each pass resolves the first mark; repeated marks ("... --") need
    // several. Bounded so a pathological line can't spin.
    for _ in 0..64 {
        match find_mark(&out) {
            Some((start, end)) => out = resolve(&out, start, end),
            None => break,
        }
    }
    out
}

/// Byte range of the first pause mark: one of `MARKS`, followed by
/// whitespace or the end of the line. A lone "-" counts only with spaces
/// on both sides ("a - b"), never inside a word.
fn find_mark(line: &str) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        for mark in MARKS {
            if !rest.starts_with(mark) {
                continue;
            }
            let end = i + mark.len();
            let followed_ok = line[end..].chars().next().is_none_or(char::is_whitespace);
            let preceded_by_space = line[..i]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace);
            // Part of a longer run of dots/dashes ("....", "---"): leave it.
            let longer = matches!(line[end..].chars().next(), Some('.' | '-'))
                || matches!(line[..i].chars().next_back(), Some('.' | '-'));
            let lone_hyphen_ok = *mark != "-" || preceded_by_space;
            if followed_ok && !longer && lone_hyphen_ok {
                return Some((i, end));
            }
            break;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

fn resolve(line: &str, start: usize, end: usize) -> String {
    let left = line[..start].trim_end();
    let right = line[end..].trim_start();

    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() || right.starts_with(['.', ',', '!', '?', ';', ':']) {
        // Trailing off at the end: close the sentence.
        let closed = if left.ends_with(['.', ',', '!', '?', ';', ':']) {
            left.to_string()
        } else {
            format!("{left}.")
        };
        return if right.is_empty() {
            closed
        } else {
            // A mark right before punctuation ("well... ?") just goes.
            format!("{left}{right}")
        };
    }
    if left.ends_with(['.', ',', '!', '?', ';', ':']) {
        return format!("{left} {right}");
    }

    let previous = last_word(left);
    let (next, rest) = split_first_word(right);
    let capitalized = next.chars().next().is_some_and(char::is_uppercase);
    let is_i = matches!(
        next.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '\''),
        "I" | "I'm" | "I'll" | "I've" | "I'd"
    );

    if CONTINUING.contains(&previous.as_str()) {
        let next = if capitalized && !is_i && COMMON.contains(&word_key(next).as_str()) {
            lowercase_first(next)
        } else {
            next.to_string()
        };
        return format!("{left} {next}{rest}");
    }
    if capitalized && !is_i {
        return format!("{left}. {next}{rest}");
    }
    format!("{left} {next}{rest}")
}

fn last_word(text: &str) -> String {
    text.split_whitespace()
        .next_back()
        .map(word_key)
        .unwrap_or_default()
}

/// Lowercased, with surrounding punctuation (but not an apostrophe) removed.
fn word_key(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_lowercase()
}

fn split_first_word(text: &str) -> (&str, &str) {
    match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], &text[i..]),
        None => (text, ""),
    }
}

fn lowercase_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::remove_pause_marks as clean;

    #[test]
    fn hesitation_after_a_continuing_word_just_joins() {
        assert_eq!(
            clean("Okay, for the... Amazon Prime"),
            "Okay, for the Amazon Prime"
        );
        assert_eq!(
            clean("move those to... our crucial"),
            "move those to our crucial"
        );
        assert_eq!(
            clean("settings here for-- Boot at startup"),
            "settings here for boot at startup"
        );
        assert_eq!(
            clean("start it when the-- computer turns on"),
            "start it when the computer turns on"
        );
        assert_eq!(
            clean("you see here, the -- or does the"),
            "you see here, the or does the"
        );
    }

    #[test]
    fn a_common_word_capitalized_after_the_pause_is_lowercased() {
        assert_eq!(
            clean("the one I have is... Do all of them"),
            "the one I have is do all of them"
        );
        assert_eq!(
            clean("I want to... The tests fail"),
            "I want to the tests fail"
        );
    }

    #[test]
    fn i_and_names_keep_their_capitals() {
        assert_eq!(clean("and... I think so"), "and I think so");
        assert_eq!(clean("open the... Xcode project"), "open the Xcode project");
    }

    #[test]
    fn a_capitalized_word_after_a_full_thought_starts_a_sentence() {
        assert_eq!(
            clean("for learning... Do we have one"),
            "for learning. Do we have one"
        );
        assert_eq!(
            clean("that works — Next we ship"),
            "that works. Next we ship"
        );
    }

    #[test]
    fn a_lowercase_continuation_just_joins() {
        assert_eq!(clean("we could... maybe ship it"), "we could maybe ship it");
        assert_eq!(clean("ship it... later today"), "ship it later today");
        assert_eq!(clean("ship it… later today"), "ship it later today");
    }

    #[test]
    fn trailing_off_ends_the_sentence() {
        assert_eq!(clean("I was thinking..."), "I was thinking.");
        assert_eq!(clean("I was thinking --"), "I was thinking.");
        assert_eq!(clean("Done."), "Done.");
    }

    #[test]
    fn a_leading_mark_is_dropped() {
        assert_eq!(clean("... so anyway"), "so anyway");
    }

    #[test]
    fn after_existing_punctuation_the_mark_just_goes() {
        assert_eq!(clean("Right, ... so we go"), "Right, so we go");
    }

    #[test]
    fn several_marks_in_a_row() {
        assert_eq!(clean("the... -- thing"), "the thing");
        assert_eq!(clean("a... b... c"), "a b c");
    }

    #[test]
    fn flags_hyphens_and_paths_are_untouched() {
        assert_eq!(clean("git commit --amend"), "git commit --amend");
        assert_eq!(clean("a well-known fix"), "a well-known fix");
        assert_eq!(clean("cd ../src"), "cd ../src");
        assert_eq!(clean("a range 1..5"), "a range 1..5");
        assert_eq!(clean("wait...."), "wait....");
        assert_eq!(clean("a --- b"), "a --- b");
    }

    #[test]
    fn list_lines_keep_their_breaks() {
        assert_eq!(
            clean("1. Fix the... tests\n2. Ship it"),
            "1. Fix the tests\n2. Ship it"
        );
    }
}
