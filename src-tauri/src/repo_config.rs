//! Team settings committed to a repo: `.mouthpiece.json` at its root.
//!
//! ```json
//! {
//!   "version": 1,
//!   "vocabulary": ["Tauri", "whisper.cpp", "sherpa-onnx"],
//!   "replacements": [{ "from": "post gress", "to": "Postgres" }]
//! }
//! ```
//!
//! Picked up whenever Mouthpiece detects you're working in that repo (the
//! same detection as names from your project). Layering: the project's own
//! names, then this file, then your personal settings, which win.
//!
//! Vocabulary only steers what Whisper listens for, so it applies on its
//! own. Replacements change what gets pasted, and a cloned repo shouldn't
//! be able to rewrite your words (into a shell command, say) without you
//! knowing, so they apply only once you've allowed this exact file in
//! Settings. Allowing is tied to the file's hash: any edit needs allowing
//! again.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE_NAME: &str = ".mouthpiece.json";
const MAX_BYTES: u64 = 64 * 1024;
const MAX_TERMS: usize = 200;
const MAX_REPLACEMENTS: usize = 100;
const MAX_CHARS: usize = 60;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub replacements: Vec<crate::corrections::Replacement>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub config: RepoConfig,
    /// sha256 of the file, hex: what "allow" is tied to.
    pub hash: String,
}

/// A repo file the user allowed, by repo folder and file hash.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Trusted {
    pub root: String,
    pub hash: String,
}

fn ok_text(s: &str) -> bool {
    let len = s.trim().chars().count();
    len > 0 && len <= MAX_CHARS && !s.chars().any(char::is_control)
}

/// Parses and checks a file's contents.
pub fn parse(bytes: &[u8]) -> Result<RepoConfig, String> {
    let config: RepoConfig =
        serde_json::from_slice(bytes).map_err(|e| format!("{FILE_NAME} isn't valid: {e}"))?;
    if config.version > 1 {
        return Err(format!(
            "{FILE_NAME} is version {}, newer than this Mouthpiece understands",
            config.version
        ));
    }
    if config.vocabulary.len() > MAX_TERMS {
        return Err(format!(
            "{FILE_NAME} has more than {MAX_TERMS} vocabulary terms"
        ));
    }
    if config.replacements.len() > MAX_REPLACEMENTS {
        return Err(format!(
            "{FILE_NAME} has more than {MAX_REPLACEMENTS} replacements"
        ));
    }
    if let Some(bad) = config.vocabulary.iter().find(|t| !ok_text(t)) {
        return Err(format!(
            "{FILE_NAME}: vocabulary term {bad:?} is empty, too long or has control characters"
        ));
    }
    if let Some(bad) = config
        .replacements
        .iter()
        .find(|r| !ok_text(&r.from) || !ok_text(&r.to))
    {
        return Err(format!(
            "{FILE_NAME}: replacement {:?} -> {:?} is empty, too long or has control characters",
            bad.from, bad.to
        ));
    }
    Ok(config)
}

pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The repo's file: `None` when there isn't one.
pub fn load(root: &Path) -> Option<Result<Loaded, String>> {
    let path = root.join(FILE_NAME);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() {
        return Some(Err(format!("{FILE_NAME} isn't a regular file")));
    }
    if meta.len() > MAX_BYTES {
        return Some(Err(format!("{FILE_NAME} is larger than 64 KB")));
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return Some(Err(format!("couldn't read {FILE_NAME}: {e}"))),
    };
    Some(parse(&bytes).map(|config| Loaded {
        config,
        hash: hash(&bytes),
    }))
}

pub fn is_trusted(trusted: &[Trusted], root: &Path, hash: &str) -> bool {
    let root = root.to_string_lossy();
    trusted.iter().any(|t| t.root == root && t.hash == hash)
}

/// Repo vocabulary ahead of the project's own names, without repeats.
pub fn merge_terms(repo: &[String], project: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(repo.len() + project.len());
    for term in repo.iter().cloned().chain(project) {
        if !out.iter().any(|t| t.eq_ignore_ascii_case(&term)) {
            out.push(term);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_good_file() {
        let c = parse(br#"{"version":1,"vocabulary":["Tauri"],"replacements":[{"from":"post gress","to":"Postgres"}]}"#).unwrap();
        assert_eq!(c.vocabulary, ["Tauri"]);
        assert_eq!(c.replacements[0].to, "Postgres");
        assert_eq!(parse(b"{}").unwrap(), RepoConfig::default());
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse(b"not json").is_err());
        assert!(parse(br#"{"version":2}"#).unwrap_err().contains("newer"));
        assert!(
            parse(br#"{"snippets":[]}"#).is_err(),
            "unknown keys are refused, not ignored"
        );
        assert!(parse(br#"{"replacements":[{"from":"deploy","to":"x\nrm -rf ~"}]}"#).is_err());
        assert!(parse(br#"{"vocabulary":[""]}"#).is_err());
        let many = format!(r#"{{"vocabulary":[{}]}}"#, vec!["\"a\""; 201].join(","));
        assert!(parse(many.as_bytes()).is_err());
    }

    #[test]
    fn trust_is_per_root_and_hash() {
        let trusted = [Trusted {
            root: "/w/app".into(),
            hash: "abc".into(),
        }];
        assert!(is_trusted(&trusted, Path::new("/w/app"), "abc"));
        assert!(!is_trusted(&trusted, Path::new("/w/app"), "abd"));
        assert!(!is_trusted(&trusted, Path::new("/w/other"), "abc"));
    }

    #[test]
    fn repo_terms_come_first_without_repeats() {
        assert_eq!(
            merge_terms(
                &["Tauri".into(), "paste_text".into()],
                vec!["paste_text".into(), "main".into()]
            ),
            ["Tauri", "paste_text", "main"]
        );
    }

    #[test]
    fn loads_from_a_folder() {
        let dir = std::env::temp_dir().join(format!("mp-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&dir).is_none());
        std::fs::write(dir.join(FILE_NAME), br#"{"vocabulary":["Tauri"]}"#).unwrap();
        let loaded = load(&dir).unwrap().unwrap();
        assert_eq!(loaded.hash, hash(br#"{"vocabulary":["Tauri"]}"#));
        std::fs::write(dir.join(FILE_NAME), b"{").unwrap();
        assert!(load(&dir).unwrap().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
