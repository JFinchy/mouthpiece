//! Structured prompts from long rambles: a dictation of 80+ words in a mode
//! with "Structure long prompts" on is reorganised by the local LLM into
//! Goal / Context / Constraints / Done when, for a coding agent.
//!
//! The rule is "keep every requirement, add nothing", and the LLM isn't
//! trusted to follow it: `check` compares the two texts and the structured
//! version is only used if every identifier, path and number survived, at
//! most a few ordinary words went missing, and little was added. Otherwise
//! the dictation is pasted as it was.

use std::collections::HashSet;

pub const MIN_WORDS: usize = 80;

pub const INSTRUCTION: &str = "You reorganise a developer's spoken request to a coding agent \
into a clear prompt. Use these headings, each on its own line, and leave out any heading with \
nothing under it:\n\
Goal: one or two sentences\n\
Context:\n- bullet points\n\
Constraints:\n- bullet points\n\
Done when:\n- bullet points\n\n\
Rules: keep every requirement, name, file, number and detail from the request, using the \
speaker's own words wherever possible. Do not add requirements, suggestions, steps or details \
that were not said. Drop only filler and repetition. Keep code names, file paths and \
@-references exactly as written. Output only the reorganised prompt, nothing else.";

const HEADINGS: &[&str] = &["goal", "context", "constraints", "done when"];

/// Words too common to count as content.
const STOPWORDS: &[&str] = &[
    "about", "after", "again", "also", "because", "been", "before", "being", "could", "does",
    "doing", "each", "from", "have", "having", "here", "into", "just", "like", "make", "more",
    "most", "much", "need", "needs", "only", "other", "over", "really", "should", "some", "something",
    "such", "than", "that", "their", "them", "then", "there", "these", "they", "thing", "things",
    "this", "those", "through", "very", "want", "wanted", "well", "were", "what", "when", "where",
    "which", "while", "will", "with", "would", "your", "yeah", "okay", "kind", "sort", "actually",
    "basically", "maybe", "think", "know", "going", "gonna", "mean", "right", "it's", "that's",
    "don't", "can't", "i'm", "we're", "you're", "let's", "goal", "context", "constraints", "done",
];

fn is_identifier_like(token: &str) -> bool {
    token.chars().any(|c| c.is_ascii_digit() || matches!(c, '_' | '/' | '@' | '`' | '.' | ':'))
        && token.chars().any(|c| c.is_alphanumeric())
        || token.chars().skip(1).any(|c| c.is_uppercase()) && token.chars().any(|c| c.is_lowercase())
}

/// Splits into tokens, trimming sentence punctuation but keeping what
/// makes a name a name (`paste_text`, `@src/a.rs:4`, `v0.6.0`).
fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| matches!(c, ',' | ';' | '!' | '?' | '(' | ')' | '"' | '\'' | '-' | '*'))
                .trim_end_matches(['.', ':'])
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// A rough stem, so "tests"/"test" and "failing"/"fails" match.
fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    for suffix in ["ing", "ed", "es", "s"] {
        if w.len() > suffix.len() + 3 {
            if let Some(base) = w.strip_suffix(suffix) {
                return base.to_string();
            }
        }
    }
    w
}

fn content_stems(text: &str) -> Vec<String> {
    tokens(text)
        .into_iter()
        .filter(|t| !is_identifier_like(t))
        .map(|t| t.to_lowercase())
        .filter(|t| t.chars().count() >= 4 && t.chars().all(|c| c.is_alphabetic() || c == '\''))
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .map(|t| stem(&t))
        .collect()
}

/// Whether `structured` is a faithful reorganisation of `original`.
pub fn check(original: &str, structured: &str) -> Result<(), String> {
    let lower = structured.to_lowercase();
    let headings = HEADINGS
        .iter()
        .filter(|h| lower.lines().any(|l| l.trim_start().trim_start_matches('#').trim().starts_with(*h)))
        .count();
    if headings < 2 {
        return Err("no structure".to_string());
    }

    // Names, paths and numbers must all come through exactly.
    let out_tokens: HashSet<String> = tokens(structured).into_iter().collect();
    for name in tokens(original).into_iter().filter(|t| is_identifier_like(t)) {
        let bare = name.trim_matches('`');
        if !out_tokens.iter().any(|t| t.trim_matches('`') == bare) && !structured.contains(bare) {
            return Err(format!("dropped a name ({} chars)", bare.chars().count()));
        }
    }

    let before: HashSet<String> = content_stems(original).into_iter().collect();
    let after: HashSet<String> = content_stems(structured).into_iter().collect();
    if before.is_empty() {
        return Err("nothing to check against".to_string());
    }
    let missing = before.difference(&after).count();
    // A few dropped words are filler ("honestly", "stuff"); more is a lost
    // requirement.
    if missing as f32 > (before.len() as f32 * 0.12).max(2.0) {
        return Err(format!("dropped {missing} of {} words", before.len()));
    }
    let added = after.difference(&before).count();
    if added as f32 > (after.len() as f32 * 0.15).max(3.0) {
        return Err(format!("added {added} words"));
    }
    Ok(())
}

/// Long enough to be worth structuring.
pub fn long_enough(text: &str) -> bool {
    text.split_whitespace().count() >= MIN_WORDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        input: String,
        output: String,
        accept: bool,
    }

    #[test]
    fn structure_golden_cases() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/fixtures/structure_cases.json")).unwrap();
        assert!(cases.len() >= 6);
        let mut failures = Vec::new();
        for case in &cases {
            let got = check(&case.input, &case.output);
            if got.is_ok() != case.accept {
                failures.push(format!("{}: {:?}", case.name, got));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn identifiers_are_recognised() {
        for t in ["paste_text", "@src/a.rs:4", "v0.6.0", "useAuthStore", "429"] {
            assert!(is_identifier_like(t), "{t}");
        }
        for t in ["Hello", "tests", "don't"] {
            assert!(!is_identifier_like(t), "{t}");
        }
    }

    #[test]
    fn only_long_dictations_qualify() {
        assert!(!long_enough("fix the bug"));
        assert!(long_enough(&"word ".repeat(80)));
    }
}
