/// File tagging, from the Smart Formatting parity backlog item (see
/// BACKLOG.md — this piece was filed separately from the rest since it's
/// really a variant of the deferred "selected-text/on-screen context"
/// work, not a punctuation command).
///
/// Wispr Flow's version works because Cursor/Windsurf are apps they have
/// a deep hook into — they can query the project's real file index and
/// insert a UI-native reference chip. Mouthpiece has neither of those:
/// it only ever knows the frontmost app's bundle ID, and delivery is
/// always a blind simulated-paste of plain text. So this can't work the
/// same way for a windowed editor.
///
/// It *can* work for terminal-based coding agents, though — Claude Code,
/// OpenCode, and Gemini CLI all accept a literal `@path/to/file` typed
/// straight into their prompt and do their own fuzzy file resolution
/// from there. We don't need a project file index at all: just recognize
/// a token that's *shaped* like a bare filename (identifier + `.` +
/// known extension) in CLI-mode dictation and prefix it with `@`, and
/// let the agent do the rest.
///
/// Scoped to CLI mode only (wired into `modes::format_as_cli`'s fallback
/// branch, not its literal shell-command directives like `git commit` —
/// tagging a file mention inside an actual commit message would corrupt
/// it). Known limitations, both accepted rather than solved:
/// - Relative paths are tagged when Whisper already wrote them as one
///   token (`src/lib.rs`), but spoken "src slash lib dot rs" isn't
///   reliably reconstructable through the existing punctuation pipeline
///   (`extract_word` doesn't have a "dot" command, so a spoken path
///   doesn't glue into one taggable token the way `~/`, `src/lib` etc.
///   do for punctuation-only text). This relies on Whisper's own
///   transcription already rendering a spoken filename like "recording
///   dot r_s" as literal `recording.rs`, which it usually does — it's
///   trained on plenty of code-adjacent text.
/// - No project awareness — this tags anything *shaped* like a file, real
///   or not. If it doesn't exist, the agent's own file resolution just
///   won't find a match, same as if you'd mistyped an `@mention` yourself.
const EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "rb", "java", "kt", "kts",
    "swift", "c", "h", "hpp", "cpp", "cc", "cs", "php", "scala", "clj", "ex", "exs", "erl", "hs",
    "lua", "sh", "bash", "zsh", "fish", "ps1", "sql", "proto", "graphql", "gql", "toml", "yaml",
    "yml", "json", "jsonc", "xml", "html", "htm", "css", "scss", "sass", "less", "md", "mdx",
    "txt", "csv", "env", "lock", "cfg", "ini", "conf", "vue", "svelte", "dart", "nim", "zig",
    "vim",
];

/// If `token` is shaped like a bare filename with a recognized extension,
/// returns the filename text with any trailing sentence-ending period
/// (added by Whisper, not spoken) stripped off. `None` if it doesn't
/// look like a file.
fn looks_like_file(token: &str) -> Option<&str> {
    let candidate = match token.strip_suffix('.') {
        // Only treat a trailing `.` as Whisper's own sentence punctuation
        // (not part of the filename) if there's still another `.` left
        // after stripping it — otherwise this is the file's only dot and
        // stripping it would break the match.
        Some(stripped) if stripped.contains('.') => stripped,
        _ => token,
    };

    let dot_idx = candidate.rfind('.')?;
    let (base, ext_with_dot) = candidate.split_at(dot_idx);
    let ext = &ext_with_dot[1..];

    if base.is_empty() || ext.is_empty() {
        return None;
    }
    // Relative paths (`src/lib.rs`) are fine; absolute ones, URLs and
    // empty segments (`a//b.rs`) aren't something an agent resolves.
    let path_ok = !base.starts_with('/') && !base.contains("//") && !base.ends_with('/');
    if !path_ok
        || !base
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '/')
    {
        return None;
    }
    if !EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
        return None;
    }

    Some(candidate)
}

fn tag_token(token: &str) -> String {
    let (core, trailing_comma) = match token.strip_suffix(',') {
        Some(stripped) => (stripped, ","),
        None => (token, ""),
    };

    match looks_like_file(core) {
        // `matched` can be shorter than `core` if a trailing sentence
        // period was stripped during matching (see `looks_like_file`) —
        // whatever's left over (just that period, if present) still
        // needs to survive into the output.
        Some(matched) => {
            let leftover = &core[matched.len()..];
            format!("@{matched}{leftover}{trailing_comma}")
        }
        None => token.to_string(),
    }
}

/// Tags any bare-filename-shaped tokens in `text` with a leading `@`, so
/// a terminal coding agent (Claude Code, OpenCode, Gemini CLI) reading
/// the pasted text picks it up as a real file reference instead of plain
/// prose.
pub fn tag_file_references(text: &str) -> String {
    // Walk runs of non-whitespace and keep the whitespace between them as-is:
    // re-joining with single spaces used to flatten dictated new lines and
    // numbered lists into one line in terminals.
    let mut out = String::with_capacity(text.len() + 8);
    let mut word_start = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(start) = word_start.take() {
                out.push_str(&tag_token(&text[start..i]));
            }
            out.push(c);
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        out.push_str(&tag_token(&text[start..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_a_bare_filename_mid_sentence() {
        assert_eq!(
            tag_file_references("look at recording.rs and fix the bug"),
            "look at @recording.rs and fix the bug"
        );
    }

    #[test]
    fn tags_multiple_files() {
        assert_eq!(
            tag_file_references("compare punctuation.rs and backtrack.rs"),
            "compare @punctuation.rs and @backtrack.rs"
        );
    }

    #[test]
    fn strips_whisper_sentence_period_but_keeps_it_after_tagging() {
        assert_eq!(
            tag_file_references("check recording.rs."),
            "check @recording.rs."
        );
    }

    #[test]
    fn keeps_trailing_comma_after_tagging() {
        assert_eq!(
            tag_file_references("open config.toml, then run it"),
            "open @config.toml, then run it"
        );
    }

    #[test]
    fn unrecognized_extension_is_left_alone() {
        assert_eq!(
            tag_file_references("email me at user.xyz please"),
            "email me at user.xyz please"
        );
    }

    #[test]
    fn does_not_misfire_on_ordinary_words_or_abbreviations() {
        assert_eq!(
            tag_file_references("for example e.g. this one"),
            "for example e.g. this one"
        );
        assert_eq!(
            tag_file_references("just a normal sentence"),
            "just a normal sentence"
        );
    }

    #[test]
    fn single_letter_extensions_still_match() {
        // .c and .h are real, common source extensions.
        assert_eq!(
            tag_file_references("edit main.c and main.h"),
            "edit @main.c and @main.h"
        );
    }

    #[test]
    fn empty_string_returns_empty() {
        assert_eq!(tag_file_references(""), "");
    }

    #[test]
    fn keeps_line_breaks_and_list_structure() {
        assert_eq!(
            tag_file_references("todo:\n1. fix paste.rs\n2. ship it"),
            "todo:\n1. fix @paste.rs\n2. ship it"
        );
    }

    #[test]
    fn already_tagged_references_are_left_alone() {
        assert_eq!(
            tag_file_references("look at @recording.rs now"),
            "look at @recording.rs now"
        );
    }

    #[test]
    fn tags_relative_paths_but_not_absolute_ones() {
        assert_eq!(
            tag_file_references("check src/lib.rs and fix it"),
            "check @src/lib.rs and fix it"
        );
        assert_eq!(
            tag_file_references("see /etc/app.conf"),
            "see /etc/app.conf"
        );
        assert_eq!(
            tag_file_references("open https://x.io/a.js"),
            "open https://x.io/a.js"
        );
    }
}
