/// Named punctuation commands, from the Smart Formatting parity backlog
/// item (see BACKLOG.md, researched against Wispr Flow's docs). Say
/// "period", "open paren", "new line", etc. and get the literal character
/// instead of the spoken word. Pure string transformation, same rationale
/// as the casing commands in `syntax.rs`: mechanical, not something that
/// benefits from an LLM's judgment, so it works instantly and regardless
/// of whether Ollama is running.
///
/// Deliberately cross-cutting like casing commands — applied to every
/// transcript before mode formatting or LLM refinement, not gated behind
/// a specific mode.
///
/// Deliberately out of scope here (see BACKLOG.md for the full list):
/// context-aware capitalization/spacing, trailing-period-by-app removal,
/// and ambiguous open/close pairs like bare "angle bracket". Also skips
/// bare "at" as an alias for "at sign" — unlike Wispr, which can afford
/// the false-positive rate given cloud-side context, a bare four-letter
/// common word is too risky to treat as a command in a local, contextless
/// pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spacing {
    /// Glues to the end of whatever precedes it; a space is expected
    /// after (period, comma, ...).
    Trailing,
    /// No space on either side — concatenates directly into the
    /// surrounding words (underscore, slash, ...).
    Glued,
    /// A space before, nothing after — opens a bracket-like construct.
    Opening,
    /// Nothing before, a space after — closes a bracket-like construct.
    Closing,
    /// Replaces with `n` newlines, consuming any surrounding spaces.
    LineBreak(usize),
    /// A space on both sides, like a word (`=>`, `==`, `&&`).
    Spaced,
}

/// `(phrase, replacement, spacing)`. Phrases are matched case-insensitively
/// against whitespace-separated tokens with their own punctuation stripped
/// (so a Whisper-inserted trailing period on the last token of an
/// utterance doesn't block a match). Checked longest-phrase-first at each
/// position, same reasoning as `syntax.rs`'s `DIRECTIVES` table.
const COMMANDS: &[(&str, &str, Spacing)] = &[
    ("period", ".", Spacing::Trailing),
    ("full stop", ".", Spacing::Trailing),
    ("comma", ",", Spacing::Trailing),
    // Whisper's most common mishearing of a spoken "comma" (seen in the
    // real-audio e2e suite); not an English word, so safe to treat as one.
    ("kama", ",", Spacing::Trailing),
    ("question mark", "?", Spacing::Trailing),
    ("exclamation point", "!", Spacing::Trailing),
    ("exclamation mark", "!", Spacing::Trailing),
    ("colon", ":", Spacing::Trailing),
    ("semicolon", ";", Spacing::Trailing),
    ("percent sign", "%", Spacing::Trailing),
    ("percentage symbol", "%", Spacing::Trailing),
    ("ellipsis", "…", Spacing::Trailing),
    ("dash", "-", Spacing::Glued),
    ("em dash", "—", Spacing::Glued),
    ("em-dash", "—", Spacing::Glued),
    ("emdash", "—", Spacing::Glued),
    ("apostrophe", "'", Spacing::Glued),
    ("single quote", "'", Spacing::Glued),
    ("asterisk", "*", Spacing::Glued),
    ("star", "*", Spacing::Glued),
    ("ampersand", "&", Spacing::Glued),
    ("slash", "/", Spacing::Glued),
    ("forward slash", "/", Spacing::Glued),
    ("backslash", "\\", Spacing::Glued),
    ("underscore", "_", Spacing::Glued),
    ("tilde", "~", Spacing::Glued),
    ("at sign", "@", Spacing::Glued),
    ("at symbol", "@", Spacing::Glued),
    ("plus", "+", Spacing::Glued),
    ("plus sign", "+", Spacing::Glued),
    ("minus", "-", Spacing::Glued),
    ("minus sign", "-", Spacing::Glued),
    ("negative", "-", Spacing::Glued),
    ("equals", "=", Spacing::Glued),
    ("equals sign", "=", Spacing::Glued),
    ("trademark", "™", Spacing::Glued),
    ("tm", "™", Spacing::Glued),
    ("registered trademark", "®", Spacing::Glued),
    ("copyright", "©", Spacing::Glued),
    ("copyright symbol", "©", Spacing::Glued),
    ("degree sign", "°", Spacing::Glued),
    ("degree symbol", "°", Spacing::Glued),
    ("degrees celsius", "°C", Spacing::Glued),
    ("degrees centigrade", "°C", Spacing::Glued),
    ("degrees fahrenheit", "°F", Spacing::Glued),
    ("degrees f", "°F", Spacing::Glued),
    ("hashtag", "#", Spacing::Opening),
    ("hash", "#", Spacing::Opening),
    ("open parenthesis", "(", Spacing::Opening),
    ("open paren", "(", Spacing::Opening),
    ("close parenthesis", ")", Spacing::Closing),
    ("close paren", ")", Spacing::Closing),
    // Code symbols. Only phrases nobody says in prose: bare "dot",
    // "arrow" or "pipe" would misfire in ordinary sentences.
    ("open bracket", "[", Spacing::Opening),
    ("open square bracket", "[", Spacing::Opening),
    ("close bracket", "]", Spacing::Closing),
    ("close square bracket", "]", Spacing::Closing),
    ("open brace", "{", Spacing::Opening),
    ("open curly brace", "{", Spacing::Opening),
    ("open curly", "{", Spacing::Opening),
    ("close brace", "}", Spacing::Closing),
    ("close curly brace", "}", Spacing::Closing),
    ("close curly", "}", Spacing::Closing),
    ("double colon", "::", Spacing::Glued),
    ("fat arrow", "=>", Spacing::Spaced),
    ("thin arrow", "->", Spacing::Spaced),
    ("double equals", "==", Spacing::Spaced),
    ("triple equals", "===", Spacing::Spaced),
    ("not equals", "!=", Spacing::Spaced),
    ("greater or equal", ">=", Spacing::Spaced),
    ("less or equal", "<=", Spacing::Spaced),
    ("double ampersand", "&&", Spacing::Spaced),
    ("double pipe", "||", Spacing::Spaced),
    ("pipe symbol", "|", Spacing::Spaced),
    ("dollar sign", "$", Spacing::Glued),
    ("new paragraph", "\n\n", Spacing::LineBreak(2)),
    ("new line", "\n", Spacing::LineBreak(1)),
    ("next line", "\n", Spacing::LineBreak(1)),
    ("line break", "\n", Spacing::LineBreak(1)),
];

const QUOTE_PHRASE: &str = "quotation mark";

/// Longest phrase in `COMMANDS`/`QUOTE_PHRASE`, in words — bounds how far
/// ahead `match_at` looks.
const MAX_PHRASE_WORDS: usize = 3;

fn normalize(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Tries to match a command phrase starting at `tokens[i]`, longest first.
/// Returns `(words_consumed, replacement, spacing)`.
fn match_at(
    tokens: &[&str],
    i: usize,
    quote_open: &mut bool,
) -> Option<(usize, &'static str, Spacing)> {
    for len in (1..=MAX_PHRASE_WORDS.min(tokens.len() - i)).rev() {
        let phrase = tokens[i..i + len]
            .iter()
            .map(|t| normalize(t))
            .collect::<Vec<_>>()
            .join(" ");

        if phrase == QUOTE_PHRASE {
            let spacing = if *quote_open {
                Spacing::Opening
            } else {
                Spacing::Closing
            };
            *quote_open = !*quote_open;
            return Some((len, "\"", spacing));
        }

        if let Some((_, replacement, spacing)) = COMMANDS.iter().find(|(p, _, _)| *p == phrase) {
            return Some((len, replacement, *spacing));
        }
    }
    None
}

fn push_word(out: &mut String, word: &str, suppress_space: &mut bool) {
    if !out.is_empty() && !*suppress_space {
        out.push(' ');
    }
    out.push_str(word);
    *suppress_space = false;
}

fn push_replacement(
    out: &mut String,
    replacement: &str,
    spacing: Spacing,
    suppress_space: &mut bool,
) {
    match spacing {
        Spacing::Trailing => {
            // Whisper's own auto-punctuation frequently lands right before
            // a spoken command word it also transcribed literally (e.g.
            // "Hello, comma" — Whisper hears the pause and inserts its own
            // comma, then transcribes "comma" too), so without this the
            // command's replacement stacks on top of it: "Hello,,". A
            // Trailing mark is always meant to be *the* terminal punctuation
            // at this position, so drop whatever's already there first
            // rather than layering a second one on.
            while matches!(out.chars().last(), Some(c) if ".,!?:;".contains(c)) {
                out.pop();
            }
            out.push_str(replacement);
            *suppress_space = false;
        }
        Spacing::Glued => {
            out.push_str(replacement);
            *suppress_space = true;
        }
        Spacing::Opening => {
            // No space after a glued symbol: "equals open paren" is "=(".
            if !out.is_empty() && !*suppress_space {
                out.push(' ');
            }
            out.push_str(replacement);
            *suppress_space = true;
        }
        Spacing::Closing => {
            out.push_str(replacement);
            *suppress_space = false;
        }
        Spacing::Spaced => push_word(out, replacement, suppress_space),
        Spacing::LineBreak(n) => {
            while out.ends_with(' ') {
                out.pop();
            }
            for _ in 0..n {
                out.push('\n');
            }
            *suppress_space = true;
        }
    }
}

/// Expands named punctuation commands in `text` into their literal
/// characters. Words that don't match any command pass through unchanged.
/// Always returns a value (unlike `syntax::try_apply_casing_command`) —
/// there's no "not a command" fallback needed, since a transcript with no
/// punctuation commands in it round-trips as itself (modulo whitespace
/// normalization).
pub fn expand_punctuation(text: &str) -> String {
    // Not `split_whitespace` — `expand_lists` may run first and glues its
    // `\n` directly onto the preceding word with no space (e.g.
    // "report\n2."), and `split_whitespace` would swallow that embedded
    // newline as a delimiter and drop it on rejoin. `tokenize` below
    // treats '\n' as its own hard-delimited token instead, so a command
    // word sitting right before a list break (e.g. "...period\n2.") still
    // gets recognized on its own rather than being glued into one
    // unmatchable blob.
    let tokens: Vec<&str> = tokenize(text);
    let mut out = String::new();
    let mut suppress_space = false;
    let mut quote_open = true;
    // A spoken "period"/"question mark" ends a sentence Whisper didn't
    // know had ended, so the next word arrives lowercase.
    let mut capitalize_next = false;
    let mut i = 0;

    while i < tokens.len() {
        if tokens[i] == "\n" {
            push_replacement(&mut out, "\n", Spacing::LineBreak(1), &mut suppress_space);
            i += 1;
        } else if let Some((consumed, replacement, spacing)) = match_at(&tokens, i, &mut quote_open)
        {
            push_replacement(&mut out, replacement, spacing, &mut suppress_space);
            capitalize_next = matches!(replacement, "." | "?" | "!");
            i += consumed;
        } else {
            if capitalize_next {
                push_word(&mut out, &capitalize_first(tokens[i]), &mut suppress_space);
            } else {
                push_word(&mut out, tokens[i], &mut suppress_space);
            }
            capitalize_next = false;
            i += 1;
        }
    }

    out
}

/// Splits on spaces like `split_whitespace`, except a literal `\n` is
/// always its own token rather than being absorbed as a delimiter — see
/// the comment in `expand_punctuation` for why that distinction matters.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut rest = text;

    loop {
        rest = rest.trim_start_matches(' ');
        if rest.is_empty() {
            break;
        }
        if let Some(stripped) = rest.strip_prefix('\n') {
            tokens.push("\n");
            rest = stripped;
            continue;
        }
        let end = rest.find([' ', '\n']).unwrap_or(rest.len());
        tokens.push(&rest[..end]);
        rest = &rest[end..];
    }

    tokens
}

const CARDINAL_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];

const ORDINAL_WORDS: &[&str] = &[
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "eleventh",
    "twelfth",
    "thirteenth",
    "fourteenth",
    "fifteenth",
    "sixteenth",
    "seventeenth",
    "eighteenth",
    "nineteenth",
    "twentieth",
];

/// Scans `tokens` for a strictly consecutive run of `family` words starting
/// at index 0 ("one"/"first"), in order but not necessarily adjacent.
/// Returns `(token_index, 1-based_value)` for each marker found.
///
/// With `at_boundaries`, a word only counts as a marker where a list item
/// could start: first in the text, after punctuation, or carrying its own
/// ("One, fix the tests."). That's what keeps "I have one question about
/// the two failing tests." from turning into a list.
fn find_sequence(tokens: &[&str], family: &[&str], at_boundaries: bool) -> Vec<(usize, usize)> {
    let ends_with_punct = |tok: &str| tok.ends_with([',', '.', ':', ';', '!', '?']);
    let mut positions = Vec::new();
    let mut expected = 1usize;
    for (i, tok) in tokens.iter().enumerate() {
        if expected > family.len() {
            break;
        }
        let at_boundary = i == 0 || ends_with_punct(tokens[i - 1]) || ends_with_punct(tok);
        if at_boundaries && !at_boundary {
            continue;
        }
        if normalize(tok) == family[expected - 1] {
            positions.push((i, expected));
            expected += 1;
        }
    }
    positions
}

/// Whisper usually writes a spoken "One, ... Two, ..." as "1. ... 2. ...",
/// all on one line. Digit markers count only with their own punctuation
/// ("1." / "1," / "1)") and at an item boundary, so "version 2" or
/// "3 tests" never start a list.
fn find_digit_sequence(tokens: &[&str]) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();
    let mut expected = 1usize;
    for (i, tok) in tokens.iter().enumerate() {
        let Some(number) = tok.strip_suffix(['.', ',', ')', ':']) else {
            continue;
        };
        let at_boundary = i == 0 || tokens[i - 1].ends_with(['.', ',', ':', ';', '!', '?']);
        if at_boundary && number == expected.to_string() {
            positions.push((i, expected));
            expected += 1;
        }
    }
    positions
}

/// Turns a spoken numbered list ("one... two...", "first... second...")
/// into a real numbered list with line breaks. Requires at least two
/// markers found in strictly consecutive order starting at 1/first — a
/// lone "one" or "first" is far too common a word on its own to treat as
/// a list start. If both families produce a run, the longer one wins.
///
/// Whisper punctuates normal speech, and a spoken list arrives with a
/// pause (a comma or period) at each marker. So when the text has any
/// punctuation, markers must sit at those boundaries (see
/// `find_sequence`); plain numbers mid-sentence are left alone. Text with
/// no punctuation at all gives no such signal, and keeps the looser match,
/// with its known false positive ("one item for two dollars").
///
/// Item text is capitalized. Doesn't punctuate the lead-in with a colon
/// the way Wispr's (context-aware, LLM-assisted) version does.
pub fn expand_lists(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return String::new();
    }

    let punctuated = text.contains(['.', ',', '!', '?', ';', ':']);
    let cardinal = find_sequence(&tokens, CARDINAL_WORDS, punctuated);
    let ordinal = find_sequence(&tokens, ORDINAL_WORDS, punctuated);
    let digits = find_digit_sequence(&tokens);
    let positions = [cardinal, ordinal, digits]
        .into_iter()
        .reduce(|best, next| if next.len() > best.len() { next } else { best })
        .unwrap_or_default();

    if positions.len() < 2 {
        return tokens.join(" ");
    }

    let mut out = String::new();
    let lead_in_end = positions[0].0;
    if lead_in_end > 0 {
        out.push_str(&tokens[..lead_in_end].join(" "));
    }

    for (idx, &(pos, value)) in positions.iter().enumerate() {
        let item_end = positions
            .get(idx + 1)
            .map(|(p, _)| *p)
            .unwrap_or(tokens.len());
        let item_text = capitalize_first(&tokens[pos + 1..item_end].join(" "));
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{value}. {item_text}"));
    }

    out
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Detects a trailing "press enter" and strips it, so the caller can paste
/// the remaining text and then simulate an Enter keystroke (see
/// `paste::press_enter`). Returns `(remaining_text, should_press_enter)`.
///
/// Only fires at the very end of the dictation — "press enter" said
/// mid-sentence stays in the text untouched, matching Wispr's own scoping
/// (a user narrating a sentence that happens to contain those words
/// shouldn't have it eaten). Tolerates Whisper's own trailing sentence
/// punctuation after "enter" (e.g. "...press enter." still matches). If
/// "press enter" is the *entire* utterance, returns an empty string but
/// still signals `true` — nothing to paste, but Enter still fires.
///
/// Doesn't replicate Wispr's context-aware auto-punctuation (their own
/// docs example shows "hello world press enter." becoming "Hello world."
/// with an inserted period) — out of scope for a pure deterministic pass,
/// same as everything else in this module.
/// True when `text` ends at a clause break, which is what lets an everyday
/// phrase like "send it" act as a command.
pub fn ends_clause(text: &str) -> bool {
    text.ends_with([',', '.', ';', '!', '?', ':'])
}

pub fn extract_press_enter(text: &str) -> (String, bool) {
    const PHRASE: &str = "press enter";
    // Aliases that are ordinary English ("and send it"), so they only count
    // after a clause break or as the whole utterance; see below.
    const GUARDED_ALIASES: &[&str] = &["send it", "submit it"];

    let trimmed = text.trim();
    let without_trailing_punct = trimmed.trim_end_matches(['.', '!', '?']).trim_end();
    let strip =
        |phrase: &str| crate::clipboard::strip_suffix_ignore_case(without_trailing_punct, phrase);

    if let Some(prefix) = strip(PHRASE) {
        if prefix.is_empty() || prefix.ends_with(' ') {
            // Whisper puts a comma at the pause before "press enter"; with
            // the phrase gone it would be the last character pasted.
            let remaining = prefix.trim_end_matches([' ', ',', ';']);
            return (remaining.to_string(), true);
        }
    }

    // "Send it" / "submit it" fire only as the final phrase *and* only after
    // a spoken or Whisper-inserted break (comma, period, ...): "Fix the
    // bug. Send it." submits, "Fix the bug and send it." is a sentence about
    // sending something and is pasted as written. Unpunctuated runs
    // ("fix the bug send it") therefore don't fire; say "press enter" or
    // pause before it.
    for alias in GUARDED_ALIASES {
        if let Some(prefix) = strip(alias) {
            if prefix.is_empty() {
                return (String::new(), true);
            }
            let before = prefix.trim_end();
            if prefix.ends_with(' ') && ends_clause(before) {
                return (before.trim_end_matches([',', ';']).to_string(), true);
            }
        }
    }

    (text.to_string(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_enter_after_non_ascii_text() {
        assert_eq!(
            extract_press_enter("İstanbul deploy, press enter"),
            ("İstanbul deploy".to_string(), true)
        );
        assert_eq!(
            extract_press_enter("Ünïcode works. Send it."),
            ("Ünïcode works.".to_string(), true)
        );
        assert_eq!(extract_press_enter("Send it"), (String::new(), true));
    }

    #[test]
    fn terminal_punctuation_glues_to_preceding_word() {
        assert_eq!(
            expand_punctuation(
                "I can't wait to see you exclamation point Let's meet at seven period"
            ),
            "I can't wait to see you! Let's meet at seven."
        );
    }

    #[test]
    fn comma_and_colon() {
        assert_eq!(
            expand_punctuation("dear team comma here's the plan colon ship it"),
            "dear team, here's the plan: ship it"
        );
    }

    #[test]
    fn glued_symbols_for_dev_use_cases() {
        // `Glued` never inserts a space on either side, even when a symbol
        // opens a new "word" (like `~` starting a path) rather than joining
        // two existing ones — that distinction needs context this pass
        // doesn't have. Known rough edge, not a bug: "cd tilde slash
        // projects" -> "cd~/projects", not "cd ~/projects".
        assert_eq!(
            expand_punctuation("cd tilde slash dot config"),
            "cd~/dot config"
        );
        assert_eq!(
            expand_punctuation("import from src slash lib"),
            "import from src/lib"
        );
        assert_eq!(expand_punctuation("x equals sign y"), "x=y");
        assert_eq!(expand_punctuation("user at sign example"), "user@example");
        assert_eq!(
            expand_punctuation("max underscore retry underscore count"),
            "max_retry_count"
        );
    }

    #[test]
    fn open_and_close_parenthesis() {
        assert_eq!(
            expand_punctuation("call foo open paren a comma b close paren"),
            "call foo (a, b)"
        );
    }

    #[test]
    fn hashtag_is_opening_only() {
        assert_eq!(
            expand_punctuation("check out hashtag coding today"),
            "check out #coding today"
        );
    }

    #[test]
    fn quotation_marks_toggle_open_and_close() {
        assert_eq!(
            expand_punctuation("she said quotation mark hello quotation mark"),
            "she said \"hello\""
        );
    }

    #[test]
    fn new_line_and_new_paragraph() {
        assert_eq!(
            expand_punctuation("reading club new line should be tomorrow"),
            "reading club\nshould be tomorrow"
        );
        assert_eq!(
            expand_punctuation("first section new paragraph second section"),
            "first section\n\nsecond section"
        );
    }

    #[test]
    fn no_commands_round_trips_with_normalized_whitespace() {
        assert_eq!(
            expand_punctuation("just a normal sentence"),
            "just a normal sentence"
        );
    }

    #[test]
    fn empty_string_returns_empty() {
        assert_eq!(expand_punctuation(""), "");
    }

    #[test]
    fn whisper_own_auto_punctuation_does_not_stack_with_a_command() {
        // Real whisper-rs output (dev-whisper manual testing, 2026-09-09)
        // for a dictated "hello comma world period new line this is a
        // test": Whisper inserted its own comma/period right next to the
        // literal command words it also transcribed, which used to
        // produce "Hello,, world." and "World,." before this fix stripped
        // the dangling mark first.
        assert_eq!(
            expand_punctuation("Hello, comma. World, period. New line. This is a test."),
            "Hello, World.\nThis is a test."
        );
    }

    #[test]
    fn tolerates_whisper_inserted_trailing_punctuation_on_command_words() {
        // Whisper's own sentence-end punctuation can land glued to the
        // last token (e.g. "period." instead of "period"); normalization
        // strips it before matching so the command still fires.
        assert_eq!(expand_punctuation("wrap it up period."), "wrap it up.");
    }

    #[test]
    fn cardinal_list_from_wispr_example() {
        assert_eq!(
            expand_lists(
                "my top goals this week are one finish the report two send the presentation"
            ),
            "my top goals this week are\n1. Finish the report\n2. Send the presentation"
        );
    }

    #[test]
    fn ordinal_list() {
        assert_eq!(
            expand_lists("first check the logs second restart the service third verify"),
            "1. Check the logs\n2. Restart the service\n3. Verify"
        );
    }

    #[test]
    fn longer_family_wins_when_both_present() {
        // Only "one" appears from the cardinal family (run length 1, not
        // enough to count), while first/second/third form a valid run —
        // ordinal wins outright, not just on a length comparison.
        assert_eq!(
            expand_lists("first grab one coffee second then get to work third done"),
            "1. Grab one coffee\n2. Then get to work\n3. Done"
        );
    }

    #[test]
    fn a_single_marker_is_not_a_list() {
        assert_eq!(
            expand_lists("i only need one thing"),
            "i only need one thing"
        );
    }

    #[test]
    fn no_markers_round_trips() {
        assert_eq!(
            expand_lists("just a normal sentence"),
            "just a normal sentence"
        );
    }

    #[test]
    fn punctuated_list_from_whisper() {
        assert_eq!(
            expand_lists("One, fix the tests. Two, ship it."),
            "1. Fix the tests.\n2. Ship it."
        );
        assert_eq!(
            expand_lists("Two things. First, check the logs. Second, restart the service."),
            "Two things.\n1. Check the logs.\n2. Restart the service."
        );
    }

    #[test]
    fn plain_numbers_in_a_punctuated_sentence_are_not_a_list() {
        for text in [
            "I have one question about the two failing tests.",
            "The first thing is the build, and the second thing is the tests.",
            "I bought one item for two dollars.",
        ] {
            assert_eq!(expand_lists(text), text);
        }
    }

    #[test]
    fn documented_false_positive_two_plain_numbers() {
        // Accepted limitation for unpunctuated text only: this isn't a
        // list, but with no punctuation there's no boundary signal. Whisper
        // punctuates this sentence, which `plain_numbers_in_a_punctuated_
        // sentence_are_not_a_list` covers.
        assert_eq!(
            expand_lists("i bought one item for two dollars"),
            "i bought\n1. Item for\n2. Dollars"
        );
    }

    #[test]
    fn composes_with_punctuation_expansion_without_losing_line_breaks() {
        // expand_lists glues its `\n` directly onto the preceding word
        // with no space, so this exercises `expand_punctuation`'s
        // `tokenize` helper, which treats that embedded newline as its
        // own token instead of silently dropping it.
        let listed = expand_lists("todo items are one fix bug period two write tests period");
        assert_eq!(
            expand_punctuation(&listed),
            "todo items are\n1. Fix bug.\n2. Write tests."
        );
    }

    #[test]
    fn press_enter_at_end_is_stripped_and_signaled() {
        assert_eq!(
            extract_press_enter("hello world press enter"),
            ("hello world".to_string(), true)
        );
    }

    #[test]
    fn press_enter_tolerates_whisper_trailing_punctuation() {
        assert_eq!(
            extract_press_enter("hello world press enter."),
            ("hello world".to_string(), true)
        );
        assert_eq!(
            extract_press_enter("hello world. press enter."),
            ("hello world.".to_string(), true)
        );
    }

    #[test]
    fn press_enter_as_entire_utterance_pastes_nothing() {
        assert_eq!(extract_press_enter("press enter"), (String::new(), true));
        assert_eq!(extract_press_enter("Press enter."), (String::new(), true));
    }

    #[test]
    fn press_enter_mid_sentence_is_left_alone() {
        assert_eq!(
            extract_press_enter("please press enter the building code"),
            ("please press enter the building code".to_string(), false)
        );
    }

    #[test]
    fn no_press_enter_returns_unchanged() {
        assert_eq!(
            extract_press_enter("just a normal sentence"),
            ("just a normal sentence".to_string(), false)
        );
    }

    #[test]
    fn press_enter_drops_the_comma_left_before_it() {
        assert_eq!(
            extract_press_enter("Run the tests, press enter."),
            ("Run the tests".to_string(), true)
        );
    }

    #[test]
    fn send_it_after_a_clause_break_acts_like_press_enter() {
        assert_eq!(
            extract_press_enter("Fix the failing test. Send it."),
            ("Fix the failing test.".to_string(), true)
        );
        assert_eq!(
            extract_press_enter("Fix the failing test, send it"),
            ("Fix the failing test".to_string(), true)
        );
        assert_eq!(
            extract_press_enter("Looks good, submit it."),
            ("Looks good".to_string(), true)
        );
        assert_eq!(extract_press_enter("Send it."), (String::new(), true));
        assert_eq!(extract_press_enter("submit it"), (String::new(), true));
    }

    #[test]
    fn send_it_inside_a_sentence_is_ordinary_text() {
        for text in [
            "I'll send it tomorrow.",
            "Send it to the team before lunch",
            "Fix the bug and send it.",
            "Please just send it",
            "the build is done so send it",
            "I will resend it",
            "Fix the bug. Don't send it tonight.",
        ] {
            assert_eq!(
                extract_press_enter(text),
                (text.to_string(), false),
                "{text}"
            );
        }
    }

    #[test]
    fn spoken_sentence_end_capitalizes_the_next_word() {
        assert_eq!(
            expand_punctuation("Hello comma world period this is a test question mark yes"),
            "Hello, world. This is a test? Yes"
        );
    }

    #[test]
    fn whisper_digit_list_gets_line_breaks() {
        assert_eq!(
            expand_lists("1. Fix the tests. 2. Ship it."),
            "1. Fix the tests.\n2. Ship it."
        );
        assert_eq!(
            expand_lists("Plan for today: 1, review the PR. 2, deploy."),
            "Plan for today:\n1. Review the PR.\n2. Deploy."
        );
    }

    #[test]
    fn digits_in_a_sentence_are_not_a_list() {
        for text in [
            "We have 3 tests and 2 failures.",
            "Bump it to version 1. Then 2 more fixes.",
        ] {
            assert_eq!(expand_lists(text), text);
        }
    }

    #[test]
    fn code_symbols() {
        assert_eq!(
            expand_punctuation("const add equals open paren a comma b close paren fat arrow a plus b"),
            "const add=(a, b) => a+b"
        );
        assert_eq!(expand_punctuation("items open bracket 0 close bracket"), "items [0]");
        assert_eq!(expand_punctuation("if x triple equals y double ampersand ok"), "if x === y && ok");
        assert_eq!(expand_punctuation("std double colon fs"), "std::fs");
        assert_eq!(expand_punctuation("fn main open curly brace close curly brace"), "fn main {}");
        assert_eq!(expand_punctuation("fn f thin arrow u32"), "fn f -> u32");
    }

    #[test]
    fn prose_with_symbol_words_is_left_alone() {
        assert_eq!(expand_punctuation("the arrow points at the dot"), "the arrow points at the dot");
        assert_eq!(expand_punctuation("pipe the output to grep"), "pipe the output to grep");
    }
}
