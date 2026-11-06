//! Names from the project being worked in (identifiers, file paths,
//! dependencies, the branch), and the matcher that puts them back into a
//! transcript: "fix the paste text function in the paste file" becomes
//! "fix the `paste_text` function in the `@src-tauri/src/paste.rs`".
//!
//! Everything here is pure (no git, no processes, no disk); `project.rs`
//! finds the repo and feeds this module its names. The names live only in
//! memory. Callers must never log them, only counts.
//!
//! The matcher prefers a missed substitution over a wrong one: exact spoken
//! forms first, then at most one or two single-letter slips in long runs,
//! and anything ambiguous (two files with the same stem, two identifiers a
//! slip apart) is left as spoken.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A function, type or constant name found in source.
    Identifier,
    /// A repo-relative file path.
    File,
    /// A package from package.json / Cargo.toml.
    Dependency,
    /// The current git branch (prompt only, never substituted).
    Branch,
}

#[derive(Clone, Debug)]
pub struct NameEntry {
    /// What gets inserted: the identifier, or the repo-relative path.
    pub text: String,
    pub kind: Kind,
    /// Higher is more relevant; see the `SCORE_*` constants.
    pub score: u32,
}

/// Rank tiers. Recently changed files and what's in them come first.
pub const SCORE_UNCOMMITTED: u32 = 1000;
/// Recent commits score between `SCORE_BRANCH` and this, newest highest.
pub const SCORE_RECENT_COMMIT: u32 = 600;
pub const SCORE_BRANCH: u32 = 500;
pub const SCORE_DEPENDENCY: u32 = 120;
pub const SCORE_TRACKED: u32 = 100;
/// A name at or above this score is "recent" for tie-breaking.
const RECENT_TIER: u32 = SCORE_BRANCH;

// --- Line references --------------------------------------------------

/// A file reference the matcher wrote: `@path`, or a path with a folder.
fn is_path_token(word: &str) -> bool {
    let core = word.trim_end_matches(|c: char| ",.;:!?)".contains(c));
    (core.starts_with('@') && core.len() > 1) || (core.contains('/') && core.contains('.'))
}

/// A line number, with any punctuation after it.
fn number_token(word: &str) -> Option<(&str, &str)> {
    let end = word
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(word.len());
    let (digits, rest) = word.split_at(end);
    (!digits.is_empty() && rest.chars().all(|c| ",.;:!?)".contains(c))).then_some((digits, rest))
}

fn is_word(word: &str, options: &[&str]) -> bool {
    let w = word.trim_end_matches(',').to_ascii_lowercase();
    options.contains(&w.as_str())
}

/// "line 42 of @src/paste.rs" and "@src/paste.rs, line 42" become
/// `@src/paste.rs:42`; "lines 10 to 20 in …" becomes `…:10-20`.
pub fn line_refs(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    // Reads "N" or "N to M" at `j`: (range text, trailing punctuation, next index).
    let range_at = |j: usize| -> Option<(String, String, usize)> {
        let (start, punct) = number_token(words.get(j)?)?;
        if punct.is_empty() {
            if let (Some(sep), Some(end_word)) = (words.get(j + 1), words.get(j + 2)) {
                if is_word(sep, &["to", "through", "-"]) {
                    if let Some((end, punct)) = number_token(end_word) {
                        return Some((format!("{start}-{end}"), punct.to_string(), j + 3));
                    }
                }
            }
        }
        Some((start.to_string(), punct.to_string(), j + 1))
    };
    while i < words.len() {
        // line N [to M] of|in PATH
        if is_word(words[i], &["line", "lines"]) {
            if let Some((range, punct, j)) = range_at(i + 1) {
                // "of the paste file" left "of the @path".
                let at = if words.get(j + 1).is_some_and(|w| is_word(w, &["the"])) {
                    j + 2
                } else {
                    j + 1
                };
                if punct.is_empty()
                    && words.get(j).is_some_and(|w| is_word(w, &["of", "in"]))
                    && words.get(at).is_some_and(|w| is_path_token(w))
                {
                    let path = words[at];
                    let core_len = path.trim_end_matches(|c: char| ",.;:!?)".contains(c)).len();
                    out.push(format!(
                        "{}:{range}{}",
                        &path[..core_len],
                        &path[core_len..]
                    ));
                    i = at + 1;
                    continue;
                }
            }
        }
        // PATH[,] [at|on] line N [to M]
        if is_path_token(words[i]) {
            let path = words[i];
            let core_len = path.trim_end_matches(|c: char| ",.;:!?)".contains(c)).len();
            let trailing = &path[core_len..];
            if trailing.is_empty() || trailing == "," {
                let mut j = i + 1;
                if words.get(j).is_some_and(|w| is_word(w, &["at", "on"])) {
                    j += 1;
                }
                if words.get(j).is_some_and(|w| is_word(w, &["line", "lines"])) {
                    if let Some((range, punct, next)) = range_at(j + 1) {
                        out.push(format!("{}:{range}{punct}", &path[..core_len]));
                        i = next;
                        continue;
                    }
                }
            }
        }
        out.push(words[i].to_string());
        i += 1;
    }
    out.join(" ")
}

// --- Spoken forms -----------------------------------------------------

/// The words someone would say for a name: `paste_text` -> [paste, text],
/// `useAuthStore` -> [use, auth, store], `HTTPServer` -> [http, server],
/// `kebab-case` -> [kebab, case]. Digits stay with the letters before them.
pub fn spoken_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for part in name.split(|c: char| !c.is_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        let chars: Vec<char> = part.chars().collect();
        let mut current = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let boundary = i > 0 && {
                let prev = chars[i - 1];
                let next = chars.get(i + 1).copied();
                // fooBar: lower/digit -> Upper starts a word.
                (c.is_uppercase() && (prev.is_lowercase() || prev.is_ascii_digit()))
                    // HTTPServer: the last capital of an acronym starts the next word.
                    || (c.is_uppercase()
                        && prev.is_uppercase()
                        && next.is_some_and(|n| n.is_lowercase()))
            };
            if boundary && !current.is_empty() {
                words.push(std::mem::take(&mut current).to_lowercase());
            }
            current.push(c);
        }
        if !current.is_empty() {
            words.push(current.to_lowercase());
        }
    }
    words
}

/// Spoken forms of a file path, each a list of words: `src/lib.rs` ->
/// "lib dot rs", "lib rs", "lib file". Only the file name is spoken. `None`
/// for files without a usable stem and extension.
pub fn file_spoken_forms(path: &str) -> Vec<Vec<String>> {
    let base = path.rsplit('/').next().unwrap_or(path);
    let Some((stem, ext)) = base.rsplit_once('.') else {
        return Vec::new();
    };
    let stem_words = spoken_words(stem);
    let ext_words = spoken_words(ext);
    if stem_words.is_empty() || ext_words.len() != 1 || stem.starts_with('.') {
        return Vec::new();
    }
    let with = |tail: &[&str]| {
        let mut words = stem_words.clone();
        words.extend(tail.iter().map(|s| s.to_string()));
        words
    };
    vec![
        with(&["dot", &ext_words[0]]),
        with(&[&ext_words[0]]),
        with(&["file"]),
    ]
}

// --- Index ------------------------------------------------------------

/// Words too common to anchor a substitution on their own.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "to", "of", "in", "on", "at", "for", "and", "or", "it",
    "this", "that", "be", "do", "can", "with", "from", "as", "by", "if", "not", "no", "so", "we",
    "i", "you", "my", "me", "up", "out", "get", "set", "has", "have", "use", "new", "all", "any",
];

/// Words next to an identifier that say "this is a code name".
const TRIGGERS: &[&str] = &[
    "function",
    "method",
    "struct",
    "type",
    "class",
    "enum",
    "trait",
    "hook",
    "component",
    "variable",
    "constant",
    "module",
    "crate",
    "package",
    "macro",
    "interface",
    "field",
    "prop",
    "called",
    "named",
    "func",
    "fn",
    "const",
    "dependency",
    "library",
];

/// An identifier needs this many letters to be substituted with no
/// code-word beside it, and this many with one.
const MIN_CHARS_BARE: usize = 12;
const MIN_CHARS_WITH_TRIGGER: usize = 7;
/// Fuzzy matching only for runs at least this long (letters).
const MIN_CHARS_FUZZY: usize = 12;
const MIN_CHARS_FUZZY_TWO: usize = 20;
/// Only words this long can have a slip.
const MIN_WORD_FUZZY: usize = 5;

struct Form {
    words: Vec<String>,
    /// How many leading words may be fuzzy (a file's "dot rs" tail may not).
    fuzzy_words: usize,
    /// Indices into `entries`, best score first.
    targets: Vec<usize>,
}

pub struct NameIndex {
    entries: Vec<NameEntry>,
    forms: Vec<Form>,
    /// First word -> indices into `forms`, longest forms first.
    by_first: HashMap<String, Vec<usize>>,
    /// File base name (lowercase) -> entry indices, for resolving a
    /// bare `paste.rs` to its real path.
    by_basename: HashMap<String, Vec<usize>>,
}

impl NameIndex {
    pub fn new(mut entries: Vec<NameEntry>) -> Self {
        // Best first, so prompt terms and ambiguity checks read in order.
        entries.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.text.cmp(&b.text)));
        entries.dedup_by(|b, a| a.text == b.text && a.kind == b.kind);

        let mut form_ids: HashMap<Vec<String>, usize> = HashMap::new();
        let mut forms: Vec<Form> = Vec::new();
        let mut by_basename: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            let candidates: Vec<(Vec<String>, usize)> = match entry.kind {
                Kind::Identifier | Kind::Dependency => {
                    let words = spoken_words(&entry.text);
                    if words.len() >= 2 {
                        let n = words.len();
                        vec![(words, n)]
                    } else {
                        Vec::new()
                    }
                }
                Kind::File => {
                    let base = entry.text.rsplit('/').next().unwrap_or(&entry.text);
                    by_basename.entry(base.to_lowercase()).or_default().push(i);
                    let stem_len =
                        spoken_words(base.rsplit_once('.').map_or(base, |(s, _)| s)).len();
                    file_spoken_forms(&entry.text)
                        .into_iter()
                        .map(|w| (w, stem_len))
                        .collect()
                }
                Kind::Branch => Vec::new(),
            };
            for (words, fuzzy_words) in candidates {
                if words.iter().all(|w| STOPWORDS.contains(&w.as_str())) {
                    continue;
                }
                let id = *form_ids.entry(words.clone()).or_insert_with(|| {
                    forms.push(Form {
                        words,
                        fuzzy_words,
                        targets: Vec::new(),
                    });
                    forms.len() - 1
                });
                forms[id].targets.push(i);
            }
        }

        let mut by_first: HashMap<String, Vec<usize>> = HashMap::new();
        for (id, form) in forms.iter().enumerate() {
            by_first.entry(form.words[0].clone()).or_default().push(id);
        }
        for ids in by_first.values_mut() {
            ids.sort_by_key(|&id| std::cmp::Reverse(forms[id].words.len()));
        }
        Self {
            entries,
            forms,
            by_first,
            by_basename,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Terms for Whisper's prompt and the LLM: highest ranked first, as
    /// they'd be written (identifiers, file names, packages, the branch).
    /// Plain lowercase words are left out; they only add noise.
    pub fn top_terms(&self, limit: usize) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for entry in &self.entries {
            if out.len() >= limit {
                break;
            }
            let term = match entry.kind {
                Kind::File => entry.text.rsplit('/').next().unwrap_or(&entry.text),
                _ => entry.text.as_str(),
            };
            let distinctive = term.len() >= 7
                || term.chars().any(|c| {
                    matches!(c, '_' | '-' | '.') || c.is_uppercase() || c.is_ascii_digit()
                });
            if term.len() < 4 || !distinctive || !seen.insert(term.to_lowercase()) {
                continue;
            }
            out.push(term.to_string());
        }
        out
    }

    /// Replaces spoken names in `text` with the real ones. `tag_files`
    /// writes file references as `@path` (terminal coding agents).
    /// Returns the new text and how many substitutions were made.
    pub fn apply(&self, text: &str, tag_files: bool) -> (String, usize) {
        if self.forms.is_empty() {
            return (text.to_string(), 0);
        }
        let tokens = tokenize(text);
        let mut out = String::with_capacity(text.len() + 16);
        let mut count = 0;
        let mut i = 0;
        // Whitespace before each token is carried in `tok.before`.
        while i < tokens.len() {
            if let Some((len, target)) = self.match_at(&tokens, i) {
                let entry = &self.entries[target];
                let first = &tokens[i];
                let last = &tokens[i + len - 1];
                out.push_str(&first.space);
                out.push_str(&first.lead);
                if entry.kind == Kind::File && tag_files {
                    out.push('@');
                }
                out.push_str(&entry.text);
                out.push_str(&last.trail);
                count += 1;
                i += len;
            } else {
                let t = &tokens[i];
                out.push_str(&t.space);
                if tag_files && self.resolve_file_token(t, &mut out) {
                    count += 1;
                } else {
                    out.push_str(&t.raw);
                }
                i += 1;
            }
        }
        out.push_str(&tail_space(text));
        if count > 0 {
            out = line_refs(&out);
        }
        (out, count)
    }

    /// `paste.rs` (or `@paste.rs`) written as one token, where the repo has
    /// exactly one file by that name: becomes its full path.
    fn resolve_file_token(&self, t: &Token, out: &mut String) -> bool {
        let core = t
            .raw
            .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '.');
        let core = core.trim_end_matches(|c: char| ".,;:!?)\"'".contains(c));
        let bare = core.trim_start_matches('@');
        if bare.contains('/') || !bare.contains('.') {
            return false;
        }
        let Some(ids) = self.by_basename.get(&bare.to_lowercase()) else {
            return false;
        };
        let Some(target) = self.pick(ids) else {
            return false;
        };
        let path = &self.entries[target].text;
        // Keep any punctuation around the token.
        let start = t.raw.find(core).unwrap_or(0);
        out.push_str(t.raw[..start].trim_end_matches('@'));
        out.push('@');
        out.push_str(path);
        out.push_str(&t.raw[start + core.len()..]);
        true
    }

    /// The single best target among candidates, or `None` if it's a toss-up:
    /// one candidate, or one clearly-ranked recent one.
    fn pick(&self, ids: &[usize]) -> Option<usize> {
        let mut sorted: Vec<usize> = ids.to_vec();
        sorted.sort_unstable(); // entries are score-ordered, so lower index is better
        match sorted.as_slice() {
            [] => None,
            [only] => Some(*only),
            [best, second, ..] => {
                let (b, s) = (self.entries[*best].score, self.entries[*second].score);
                (b >= RECENT_TIER && b > s).then_some(*best)
            }
        }
    }

    /// The longest form matching at token `i`: (token count, entry index).
    fn match_at(&self, tokens: &[Token], i: usize) -> Option<(usize, usize)> {
        let first = tokens[i].word.as_deref()?;
        let form_ids = self.by_first.get(first)?;
        let mut best_len = 0;
        let mut found: Vec<(usize, usize)> = Vec::new(); // (form id, distance)
        for &id in form_ids {
            let form = &self.forms[id];
            let n = form.words.len();
            if best_len > n {
                break; // forms are longest first
            }
            if i + n > tokens.len() {
                continue;
            }
            let Some(distance) = run_distance(tokens, i, form) else {
                continue;
            };
            if !self.context_ok(tokens, i, n, form) {
                continue;
            }
            best_len = n;
            found.push((id, distance));
        }
        let min_distance = found.iter().map(|&(_, d)| d).min()?;
        let mut best = found.iter().filter(|&&(_, d)| d == min_distance);
        let (form_id, _) = *best.next()?;
        if best.next().is_some() {
            return None; // two different names equally close
        }
        let target = self.pick(&self.forms[form_id].targets)?;
        Some((self.forms[form_id].words.len(), target))
    }

    /// The precision gate for identifiers: a long enough name, or a
    /// code-word right beside it. Files are distinctive on their own tail.
    fn context_ok(&self, tokens: &[Token], i: usize, n: usize, form: &Form) -> bool {
        let entry = &self.entries[form.targets[0]];
        if entry.kind == Kind::File {
            return true;
        }
        let chars: usize = form.words.iter().map(|w| w.chars().count()).sum();
        if chars >= MIN_CHARS_BARE {
            return true;
        }
        if chars < MIN_CHARS_WITH_TRIGGER {
            return false;
        }
        let is_trigger = |t: Option<&Token>| {
            t.and_then(|t| t.word.as_deref())
                .is_some_and(|w| TRIGGERS.contains(&w))
        };
        let before = i.checked_sub(1).and_then(|p| tokens.get(p));
        is_trigger(before) || is_trigger(tokens.get(i + n))
    }
}

/// Edit distance between a run of transcript words and a form, or `None`
/// if they don't match. 0 is exact.
fn run_distance(tokens: &[Token], i: usize, form: &Form) -> Option<usize> {
    let n = form.words.len();
    let mut slips = 0;
    let mut fuzzy_chars = 0;
    for (k, want) in form.words.iter().enumerate() {
        let got = tokens[i + k].word.as_deref()?;
        if got == want {
            if k < form.fuzzy_words {
                fuzzy_chars += want.chars().count();
            }
            continue;
        }
        if k >= form.fuzzy_words || want.chars().count() < MIN_WORD_FUZZY || k == 0 {
            return None; // the first word is how we found the form; it's exact
        }
        if edit_distance_within_one(got, want) {
            slips += 1;
            fuzzy_chars += want.chars().count();
        } else {
            return None;
        }
    }
    if slips > 0 {
        let allowed = if fuzzy_chars >= MIN_CHARS_FUZZY_TWO {
            2
        } else if fuzzy_chars >= MIN_CHARS_FUZZY {
            1
        } else {
            0
        };
        if slips > allowed {
            return None;
        }
    }
    // A run must read as one phrase: nothing but spaces between the words.
    // (Checked last: most forms already failed on the words above.)
    for k in 0..n {
        if k > 0 && !tokens[i + k].lead.is_empty() {
            return None;
        }
        if k + 1 < n && !tokens[i + k].trail.is_empty() {
            return None;
        }
    }
    Some(slips)
}

/// True if `a` and `b` differ by exactly one insertion, deletion or
/// substitution (they are known to be unequal).
fn edit_distance_within_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (ra, rb) = (a.len() - prefix - suffix, b.len() - prefix - suffix);
    ra <= 1 && rb <= 1 && (ra + rb) >= 1
}

// --- Tokens -----------------------------------------------------------

struct Token {
    /// Whitespace before the token.
    space: String,
    /// Punctuation before and after the word.
    lead: String,
    trail: String,
    /// The whole token as written.
    raw: String,
    /// The lowercase word, if the token is plain letters/digits/apostrophes
    /// (a `paste_text` or `src/x.rs` token is not matchable).
    word: Option<String>,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let space_len = rest.len() - rest.trim_start().len();
        let (space, after) = rest.split_at(space_len);
        if after.is_empty() {
            break;
        }
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let (raw, next) = after.split_at(end);
        rest = next;

        let lead_len = raw.len() - raw.trim_start_matches(is_edge_punct).len();
        let core_and_trail = &raw[lead_len..];
        let core_len = core_and_trail.trim_end_matches(is_edge_punct).len();
        let core = &core_and_trail[..core_len];
        let word = (!core.is_empty()
            && core
                .chars()
                .all(|c| c.is_alphanumeric() || c == '\'' || c == '’'))
        .then(|| core.to_lowercase());
        tokens.push(Token {
            space: space.to_string(),
            lead: raw[..lead_len].to_string(),
            trail: core_and_trail[core_len..].to_string(),
            raw: raw.to_string(),
            word,
        });
    }
    tokens
}

fn is_edge_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | '(' | ')' | '"' | '\'' | '[' | ']' | '{' | '}' | '`'
    )
}

fn tail_space(text: &str) -> String {
    text[text.trim_end().len()..].to_string()
}

// --- Prompt -----------------------------------------------------------

/// Whisper reads about 224 tokens of prompt and keeps the *end* when it's
/// longer, so the user's own terms go last and project terms fill what's
/// left of the budget before them.
const PROMPT_TOKEN_BUDGET: usize = 200;

fn estimate_tokens(term: &str) -> usize {
    // Identifiers split into many pieces; ~3 chars per token plus the comma.
    term.chars().count().div_ceil(3) + 1
}

/// The initial prompt: project terms (as many as fit) then the user's
/// vocabulary, comma separated.
pub fn whisper_prompt(project_terms: &[String], user_vocabulary: &[String]) -> String {
    let user_cost: usize = user_vocabulary.iter().map(|t| estimate_tokens(t)).sum();
    let mut room = PROMPT_TOKEN_BUDGET.saturating_sub(user_cost);
    let mut chosen: Vec<&str> = Vec::new();
    for term in project_terms.iter().take(40) {
        let cost = estimate_tokens(term);
        if cost > room {
            break;
        }
        room -= cost;
        chosen.push(term);
    }
    chosen
        .into_iter()
        .chain(user_vocabulary.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        spoken_words(s)
    }

    fn entry(text: &str, kind: Kind, score: u32) -> NameEntry {
        NameEntry {
            text: text.to_string(),
            kind,
            score,
        }
    }

    fn index(names: &[(&str, Kind, u32)]) -> NameIndex {
        NameIndex::new(names.iter().map(|&(t, k, s)| entry(t, k, s)).collect())
    }

    fn sample() -> NameIndex {
        index(&[
            ("paste_text", Kind::Identifier, SCORE_UNCOMMITTED),
            ("src-tauri/src/paste.rs", Kind::File, SCORE_UNCOMMITTED),
            ("src-tauri/src/file_tagging.rs", Kind::File, SCORE_TRACKED),
            ("src/lib/useAuthStore.ts", Kind::File, SCORE_TRACKED),
            ("useAuthStore", Kind::Identifier, SCORE_TRACKED),
            ("tag_file_references", Kind::Identifier, SCORE_TRACKED),
            ("handle_request", Kind::Identifier, SCORE_TRACKED),
            ("handle_response", Kind::Identifier, SCORE_TRACKED),
            ("process_transcript", Kind::Identifier, SCORE_TRACKED),
            ("src/a/mod.rs", Kind::File, SCORE_TRACKED),
            ("src/b/mod.rs", Kind::File, SCORE_TRACKED),
            ("tauri-plugin-opener", Kind::Dependency, SCORE_DEPENDENCY),
        ])
    }

    fn run(text: &str) -> String {
        sample().apply(text, true).0
    }

    #[test]
    fn spoken_forms_split_every_naming_style() {
        assert_eq!(words("paste_text"), ["paste", "text"]);
        assert_eq!(words("useAuthStore"), ["use", "auth", "store"]);
        assert_eq!(words("HTTPServer"), ["http", "server"]);
        assert_eq!(words("kebab-case"), ["kebab", "case"]);
        assert_eq!(words("MAX_RETRY_COUNT"), ["max", "retry", "count"]);
        assert_eq!(words("parseV2Response"), ["parse", "v2", "response"]);
        assert_eq!(words("PascalCase"), ["pascal", "case"]);
        assert_eq!(words("utils"), ["utils"]);
        assert!(words("___").is_empty());
    }

    #[test]
    fn file_forms_cover_dot_ext_and_file() {
        assert_eq!(
            file_spoken_forms("src/lib.rs"),
            vec![
                vec!["lib", "dot", "rs"],
                vec!["lib", "rs"],
                vec!["lib", "file"]
            ]
        );
        assert_eq!(
            file_spoken_forms("a/file_tagging.rs")[2],
            ["file", "tagging", "file"]
        );
        assert!(file_spoken_forms("Makefile").is_empty());
        assert!(file_spoken_forms(".gitignore").is_empty());
    }

    #[test]
    fn replaces_identifier_and_file_in_the_example_sentence() {
        assert_eq!(
            run("fix the paste text function in the paste file"),
            "fix the paste_text function in the @src-tauri/src/paste.rs"
        );
    }

    #[test]
    fn spoken_dot_rs_resolves_to_the_real_path() {
        assert_eq!(
            run("look at file tagging dot rs, then paste dot rs."),
            "look at @src-tauri/src/file_tagging.rs, then @src-tauri/src/paste.rs."
        );
    }

    #[test]
    fn line_numbers_join_the_file_reference() {
        assert_eq!(
            run("look at line 42 of the paste file"),
            "look at @src-tauri/src/paste.rs:42"
        );
        assert_eq!(
            run("check the paste file, line 7, it panics"),
            "check the @src-tauri/src/paste.rs:7, it panics"
        );
        assert_eq!(
            run("lines 10 to 20 in the paste file."),
            "@src-tauri/src/paste.rs:10-20."
        );
        assert_eq!(
            run("the paste file at line 3"),
            "the @src-tauri/src/paste.rs:3"
        );
    }

    #[test]
    fn line_refs_leave_other_numbers_alone() {
        assert_eq!(line_refs("line 42 of the poem"), "line 42 of the poem");
        assert_eq!(line_refs("@a.rs has 3 lines"), "@a.rs has 3 lines");
        assert_eq!(line_refs("@a.rs. Line 4 is fine"), "@a.rs. Line 4 is fine");
    }

    #[test]
    fn files_are_plain_paths_outside_terminal_agents() {
        let (out, n) = sample().apply("open the paste file", false);
        assert_eq!((out.as_str(), n), ("open the src-tauri/src/paste.rs", 1));
    }

    #[test]
    fn bare_token_filenames_resolve_to_paths() {
        assert_eq!(
            run("check paste.rs now"),
            "check @src-tauri/src/paste.rs now"
        );
        assert_eq!(run("check @paste.rs."), "check @src-tauri/src/paste.rs.");
        // Already a path, or not in the repo: untouched.
        assert_eq!(run("check src/x/paste.rs"), "check src/x/paste.rs");
        assert_eq!(run("check other.rs"), "check other.rs");
    }

    #[test]
    fn camel_case_identifier_from_a_long_phrase_needs_no_code_word() {
        assert_eq!(run("call use auth store again"), "call useAuthStore again");
    }

    #[test]
    fn short_identifier_needs_a_code_word_beside_it() {
        // "paste text" is 9 letters: replaced with "function" next to it...
        assert_eq!(run("the paste text function"), "the paste_text function");
        assert_eq!(
            run("a function called paste text"),
            "a function called paste_text"
        );
        // ...and left alone in ordinary prose.
        assert_eq!(
            run("just paste text into the box"),
            "just paste text into the box"
        );
    }

    #[test]
    fn keeps_punctuation_and_whitespace_around_a_match() {
        assert_eq!(
            run("what does (process transcript) do?\nAnd then tag file references, ok"),
            "what does (process_transcript) do?\nAnd then tag_file_references, ok"
        );
    }

    #[test]
    fn fuzzy_catches_a_one_letter_slip_in_a_long_name() {
        assert_eq!(
            run("rename process transcripts"),
            "rename process_transcript"
        );
        assert_eq!(run("call tag file reference"), "call tag_file_references");
    }

    #[test]
    fn near_misses_are_not_replaced() {
        // Different real word, same length: first word must be exact...
        assert_eq!(run("paste test function"), "paste test function");
        // ...slip in a short word never counts...
        assert_eq!(run("call tag fill references"), "call tag fill references");
        // ...too short for a slip.
        assert_eq!(run("the pasta text function"), "the pasta text function");
        // Two names equally close are ambiguous, so neither wins.
        let idx = index(&[
            ("load_items_cache", Kind::Identifier, SCORE_TRACKED),
            ("load_itemz_cache", Kind::Identifier, SCORE_TRACKED),
        ]);
        assert_eq!(idx.apply("call load itemx cache", true).1, 0);
        assert_eq!(
            idx.apply("call load items cache", true).0,
            "call load_items_cache"
        );
        // Words inside a run can't have punctuation between them.
        assert_eq!(run("paste. text function"), "paste. text function");
        // Part of a code token is not a spoken phrase.
        assert_eq!(run("use my_auth store"), "use my_auth store");
    }

    #[test]
    fn ambiguous_file_stems_are_left_as_spoken() {
        assert_eq!(run("open the mod file"), "open the mod file");
        let idx = index(&[
            ("src/a/mod.rs", Kind::File, SCORE_UNCOMMITTED),
            ("src/b/mod.rs", Kind::File, SCORE_TRACKED),
        ]);
        // One is clearly the recent one.
        assert_eq!(idx.apply("open mod rs", true).0, "open @src/a/mod.rs");
    }

    #[test]
    fn stopword_only_names_are_never_indexed() {
        let idx = index(&[("is_the", Kind::Identifier, SCORE_TRACKED)]);
        assert_eq!(idx.apply("it is the function", true).1, 0);
    }

    #[test]
    fn dependencies_match_like_identifiers() {
        assert_eq!(
            run("add the tauri plugin opener package"),
            "add the tauri-plugin-opener package"
        );
    }

    #[test]
    fn text_without_matches_is_returned_unchanged() {
        let text = "  Let's meet at three.\n\nThanks!  ";
        assert_eq!(sample().apply(text, true), (text.to_string(), 0));
    }

    #[test]
    fn top_terms_follow_rank_and_skip_plain_words() {
        let idx = index(&[
            ("tracked_helper", Kind::Identifier, SCORE_TRACKED),
            ("src/x/recent_file.rs", Kind::File, SCORE_UNCOMMITTED),
            ("config", Kind::Identifier, SCORE_UNCOMMITTED),
            ("HotPath", Kind::Identifier, SCORE_RECENT_COMMIT),
            ("hotpath", Kind::Identifier, SCORE_TRACKED),
        ]);
        assert_eq!(
            idx.top_terms(10),
            ["recent_file.rs", "HotPath", "tracked_helper"]
        );
        assert_eq!(idx.top_terms(1), ["recent_file.rs"]);
    }

    #[test]
    fn whisper_prompt_keeps_user_terms_and_respects_the_budget() {
        let user = vec!["kubectl".to_string(), "Tauri".to_string()];
        let project: Vec<String> = (0..100).map(|i| format!("project_name_{i:03}")).collect();
        let prompt = whisper_prompt(&project, &user);
        assert!(prompt.ends_with("kubectl, Tauri"));
        assert!(prompt.starts_with("project_name_000, project_name_001"));
        let terms = prompt.split(", ").count();
        assert!(terms <= 42, "{terms} terms");
        // A huge user vocabulary crowds the project out, never the reverse.
        let big: Vec<String> = (0..200).map(|i| format!("term{i}")).collect();
        assert_eq!(whisper_prompt(&project, &big), big.join(", "));
        assert_eq!(whisper_prompt(&[], &user), "kubectl, Tauri");
    }

    #[test]
    fn matcher_is_fast_on_a_big_repo() {
        let mut entries = Vec::new();
        for i in 0..20_000 {
            entries.push(entry(
                &format!("src/module_{i}/handler_{}.rs", i % 97),
                Kind::File,
                SCORE_TRACKED,
            ));
            entries.push(entry(
                &format!("do_thing_{i}_now"),
                Kind::Identifier,
                SCORE_TRACKED,
            ));
        }
        let idx = NameIndex::new(entries);
        let text = "so I was thinking we should fix the handler file and then also look at \
                    do thing 42 now because the module has a bug in the parsing of requests \
                    and the retry logic should back off a little before the next attempt";
        let started = std::time::Instant::now();
        for _ in 0..20 {
            idx.apply(text, true);
        }
        let per_call = started.elapsed() / 20;
        // Debug builds run this an order of magnitude slower than release.
        let limit = if cfg!(debug_assertions) { 25 } else { 5 };
        assert!(per_call.as_millis() < limit, "{per_call:?} per call");
    }

    #[test]
    fn edit_distance_helper() {
        assert!(edit_distance_within_one("transcripts", "transcript"));
        assert!(edit_distance_within_one("reference", "references"));
        assert!(edit_distance_within_one("pasta", "paste"));
        assert!(!edit_distance_within_one("abc", "xyz"));
        assert!(!edit_distance_within_one("abcd", "abdc"));
    }
}
