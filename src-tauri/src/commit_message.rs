//! "Commit message: fixed the race in paste" writes a git commit message
//! from the project's staged changes, steered by what you said. The local
//! LLM writes it; with nothing staged, no project or no LLM, your words
//! are pasted as spoken.

const LEADS: &[&str] = &["write a commit message", "commit message"];

/// The notes after a leading "commit message", or `None` if the dictation
/// doesn't start with it. A bare "commit message" gives empty notes.
pub fn try_extract(text: &str) -> Option<String> {
    let trimmed = text.trim();
    for lead in LEADS {
        let Some(head) = trimmed.get(..lead.len()) else {
            continue;
        };
        if !head.eq_ignore_ascii_case(lead) {
            continue;
        }
        let rest = &trimmed[lead.len()..];
        // "commit message: ...", "commit message, ...", "commit message."
        // or "commit message for ...", but not "commit messages are ...".
        let rest = if let Some(r) = rest.strip_prefix([':', ',', '.', ';', '-']) {
            r
        } else if rest.is_empty() {
            rest
        } else if let Some(r) = rest
            .strip_prefix(' ')
            .and_then(|r| r.strip_prefix("for ").or_else(|| r.strip_prefix("about ")))
        {
            r
        } else {
            continue;
        };
        return Some(rest.trim().trim_end_matches('.').trim().to_string());
    }
    None
}

pub const INSTRUCTION: &str = "Write a git commit message for the staged changes below. \
First line: an imperative summary of at most 72 characters, no trailing period. If the change \
needs explaining, add a blank line and a short body wrapped at 72 characters. Use the \
developer's notes for the intent and wording; describe only what the diff and notes show. \
Output only the commit message.";

/// Trims what models wrap messages in.
pub fn clean(answer: &str) -> String {
    let answer = answer.trim().trim_matches('`').trim();
    let answer = answer
        .strip_prefix("Commit message:")
        .unwrap_or(answer)
        .trim();
    answer.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_notes_after_the_lead() {
        assert_eq!(try_extract("Commit message: fixed the race in paste.").as_deref(), Some("fixed the race in paste"));
        assert_eq!(try_extract("commit message for the new hook script").as_deref(), Some("the new hook script"));
        assert_eq!(try_extract("Write a commit message.").as_deref(), Some(""));
        assert_eq!(try_extract("Commit message").as_deref(), Some(""));
    }

    #[test]
    fn ordinary_sentences_are_left_alone() {
        assert_eq!(try_extract("Commit messages should be short."), None);
        assert_eq!(try_extract("The commit message is wrong."), None);
        assert_eq!(try_extract("commit messageboard"), None);
    }

    #[test]
    fn cleans_model_wrapping() {
        assert_eq!(clean("```\nCommit message: Fix paste race\n```"), "Fix paste race");
    }
}
