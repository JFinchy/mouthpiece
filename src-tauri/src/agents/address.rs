//! Talking to a particular agent by its folder name: "to esc pause, rerun
//! the tests", "tell dev whisper to commit", "hey api: what's left?".
//!
//! Only agents Mouthpiece has heard from (through the hook) can be
//! addressed, and only when the name picks out exactly one of them, so an
//! ordinary sentence that happens to start with "to" is pasted as spoken.

/// Words that start an address. "to" and "hey" need a comma or colon after
/// the name, because "to the store" or "hey there" would otherwise read as
/// one; "tell" doesn't, since "tell <folder> ..." is unambiguous.
const LEADS: &[(&str, bool)] = &[("to", true), ("hey", true), ("tell", false), ("ask", false)];

/// Longest spoken name tried, in words.
const MAX_NAME_WORDS: usize = 6;

#[derive(Debug, PartialEq)]
pub struct Addressed {
    /// Index into the candidate folders.
    pub index: usize,
    /// What's left to send, with the address removed.
    pub rest: String,
}

/// Splits a folder name into lowercase words: "dev-whisper_escPause2" ->
/// ["dev", "whisper", "esc", "pause", "2"].
pub fn folder_words(folder: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev: Option<char> = None;
    for c in folder.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev = None;
            continue;
        }
        let boundary = match prev {
            Some(p) => {
                (p.is_lowercase() && c.is_uppercase())
                    || (p.is_alphabetic() && c.is_ascii_digit())
                    || (p.is_ascii_digit() && c.is_alphabetic())
            }
            None => false,
        };
        if boundary && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.extend(c.to_lowercase());
        prev = Some(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// One spoken word, with whether a comma or colon followed it.
struct Token {
    words: Vec<String>,
    /// Byte offset in the text just past this token (and its punctuation).
    end: usize,
    clause_break: bool,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut offset = 0;
    for raw in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += raw.len();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let clause_break = trimmed.ends_with([',', ':', ';', '—', '-']) && trimmed.len() > 1;
        let words = folder_words(trimmed);
        if words.is_empty() {
            continue;
        }
        out.push(Token {
            words,
            end: start + raw.trim_end().len(),
            clause_break,
        });
    }
    out
}

/// Whether the spoken words name this folder: all of its words, its
/// trailing words ("esc pause" for "dev-whisper-esc-pause"), or the same
/// letters run together ("devwhisper" for "dev-whisper").
fn names(spoken: &[String], folder: &[String]) -> bool {
    if spoken.is_empty() || folder.is_empty() {
        return false;
    }
    if folder.ends_with(spoken) {
        // A lone trailing word that's very short ("2", "app") is too easy to
        // say by accident unless it's the whole name.
        return spoken.len() == folder.len() || spoken.concat().len() >= 3;
    }
    let joined = spoken.concat();
    let folder_joined = folder.concat();
    joined == folder_joined
        || (joined.len() >= 4
            && folder_joined.ends_with(&joined)
            && folder
                .iter()
                .rev()
                .scan(String::new(), |acc, w| {
                    *acc = format!("{w}{acc}");
                    Some(acc.clone())
                })
                .any(|tail| tail == joined))
}

/// Finds an address at the start of `text` naming exactly one of
/// `folders`. Prefers the longest name, so "to dev whisper esc pause"
/// picks the esc-pause worktree over "dev-whisper".
pub fn find(text: &str, folders: &[String]) -> Option<Addressed> {
    let toks = tokens(text);
    let first = toks.first()?;
    let lead = first.words.join(" ");
    let &(_, needs_break) = LEADS.iter().find(|(word, _)| *word == lead)?;
    let folder_words: Vec<Vec<String>> = folders.iter().map(|f| folder_words(f)).collect();

    // Flatten the spoken words after the lead, remembering which token each
    // came from, so a hyphenated "esc-pause" counts as two words.
    let mut spoken: Vec<(String, usize)> = Vec::new();
    for (i, tok) in toks.iter().enumerate().skip(1) {
        for w in &tok.words {
            spoken.push((w.clone(), i));
        }
        if spoken.len() >= MAX_NAME_WORDS {
            break;
        }
    }

    for len in (1..=spoken.len().min(MAX_NAME_WORDS)).rev() {
        let last_token = spoken[len - 1].1;
        // The name must end on a token boundary.
        if spoken.get(len).is_some_and(|(_, t)| *t == last_token) {
            continue;
        }
        let tok = &toks[last_token];
        if needs_break && !tok.clause_break {
            continue;
        }
        let candidate: Vec<String> = spoken[..len].iter().map(|(w, _)| w.clone()).collect();
        let hits: Vec<usize> = folder_words
            .iter()
            .enumerate()
            .filter(|(_, f)| names(&candidate, f))
            .map(|(i, _)| i)
            .collect();
        let index = match hits[..] {
            [only] => only,
            // The same folder twice (two sessions there) is still one place.
            [a, ..] if hits.iter().all(|&h| folders[h] == folders[a]) => a,
            _ => continue,
        };
        let rest = text[tok.end..]
            .trim_start_matches(|c: char| {
                c.is_whitespace() || matches!(c, ',' | ':' | ';' | '—' | '-')
            })
            .trim();
        // "tell api to run the tests" -> "run the tests".
        let rest = if !needs_break {
            strip_leading_to(rest)
        } else {
            rest
        };
        if rest.is_empty() {
            return None;
        }
        return Some(Addressed {
            index,
            rest: capitalize_first(rest),
        });
    }
    None
}

fn strip_leading_to(rest: &str) -> &str {
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("to ") {
        rest[3..].trim_start()
    } else {
        rest
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_folder_names_into_words() {
        assert_eq!(
            folder_words("dev-whisper_escPause2"),
            ["dev", "whisper", "esc", "pause", "2"]
        );
        assert_eq!(folder_words("API"), ["api"]);
    }

    #[test]
    fn to_with_a_comma_addresses_by_trailing_words() {
        let f = folders(&["dev-whisper", "dev-whisper-esc-pause"]);
        let hit = find("To esc pause, rerun the tests.", &f).unwrap();
        assert_eq!(
            hit,
            Addressed {
                index: 1,
                rest: "Rerun the tests.".into()
            }
        );
    }

    #[test]
    fn longest_name_wins() {
        let f = folders(&["dev-whisper", "dev-whisper-esc-pause"]);
        assert_eq!(find("To dev whisper, commit it", &f).unwrap().index, 0);
        assert_eq!(
            find("To dev whisper esc pause: commit it", &f)
                .unwrap()
                .index,
            1
        );
    }

    #[test]
    fn hyphenated_and_run_together_names_match() {
        let f = folders(&["dev-whisper-esc-pause"]);
        assert_eq!(find("to esc-pause: go", &f).unwrap().rest, "Go");
        assert_eq!(find("to escpause, go", &f).unwrap().rest, "Go");
    }

    #[test]
    fn tell_needs_no_comma_and_drops_a_following_to() {
        let f = folders(&["api", "web"]);
        let hit = find("Tell api to run the migrations", &f).unwrap();
        assert_eq!(
            hit,
            Addressed {
                index: 0,
                rest: "Run the migrations".into()
            }
        );
    }

    #[test]
    fn ordinary_sentences_are_left_alone() {
        let f = folders(&["api", "web", "store"]);
        assert_eq!(find("To be honest, I'm not sure", &f), None);
        assert_eq!(find("to the store and back", &f), None);
        // No comma after "to store": could be prose.
        assert_eq!(find("to store the value we need a map", &f), None);
        assert_eq!(find("Hey there, how's it going", &f), None);
    }

    #[test]
    fn ambiguous_names_are_not_routed() {
        let f = folders(&["frontend-app", "backend-app"]);
        assert_eq!(find("to app, deploy", &f), None);
        assert_eq!(find("to frontend app, deploy", &f).unwrap().index, 0);
    }

    #[test]
    fn same_folder_twice_still_routes() {
        let f = folders(&["api", "api"]);
        assert_eq!(find("to api, status?", &f).unwrap().index, 0);
    }

    #[test]
    fn needs_something_to_send() {
        let f = folders(&["api"]);
        assert_eq!(find("to api,", &f), None);
    }

    #[test]
    fn short_trailing_words_need_the_whole_name() {
        let f = folders(&["worktree-2"]);
        assert_eq!(find("to 2, go", &f), None);
        assert_eq!(find("to worktree 2, go", &f).unwrap().index, 0);
    }
}
