//! Finds the project you're dictating into and reads its names, for
//! `names.rs` to use. When the frontmost app is a terminal or editor we
//! work out which git repo it's in (a terminal's shell working directory,
//! or an editor's window title / recent-folder state), then build a
//! `NameIndex` from `git ls-files`, recent changes, dependencies and
//! declarations found in source.
//!
//! Nothing here blocks a dictation: detection and indexing run on a
//! background thread started when recording starts, and the pipeline uses
//! whatever is ready by then. Names stay in memory. Logs carry counts and
//! timings only, never names or paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::names::{
    Kind, NameEntry, NameIndex, SCORE_BRANCH, SCORE_DEPENDENCY, SCORE_RECENT_COMMIT, SCORE_TRACKED,
    SCORE_UNCOMMITTED,
};

/// A repo's names, ready to use.
pub struct Project {
    /// The repo folder's name, shown in Settings.
    pub name: String,
    pub index: NameIndex,
    /// The top names, for Whisper's prompt and the LLM.
    pub terms: Vec<String>,
}

const TRACKED_FILE_CAP: usize = 20_000;
const SOURCE_FILES_SCANNED: usize = 4_000;
const SOURCE_FILE_MAX_BYTES: u64 = 200 * 1024;
const SCAN_BUDGET: Duration = Duration::from_secs(4);
const IDENTIFIER_CAP: usize = 20_000;
const PROMPT_TERMS: usize = 40;
/// How long a finished index is served before a background refresh.
const INDEX_TTL: Duration = Duration::from_secs(90);
/// How long "this app's shell is in that folder" is trusted.
const DETECT_TTL: Duration = Duration::from_secs(5);
/// Repo indexes kept in memory at once.
const MAX_INDEXES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AppKind {
    Terminal,
    Editor,
}

/// Terminal emulators, the one list shared by project detection, undo and
/// "attach the error".
pub fn is_terminal(bundle_id: &str) -> bool {
    classify(bundle_id) == Some(AppKind::Terminal)
}

fn classify(bundle_id: &str) -> Option<AppKind> {
    const TERMINALS: &[&str] = &[
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "dev.warp.Warp-Stable",
        "dev.warp.Warp",
        "com.github.wez.wezterm",
        "net.kovidgoyal.kitty",
        "org.alacritty",
        "com.mitchellh.ghostty",
        "co.zeit.hyper",
        "io.alacritty",
    ];
    const EDITORS: &[&str] = &[
        "com.microsoft.VSCode",
        "com.microsoft.VSCodeInsiders",
        "com.todesktop.230313mzl4w4u92", // Cursor
        "com.exafunction.windsurf",
        "com.vscodium",
        "dev.zed.Zed",
        "dev.zed.Zed-Preview",
        "com.sublimetext.4",
        "com.sublimetext.3",
    ];
    if TERMINALS.contains(&bundle_id) {
        Some(AppKind::Terminal)
    } else if EDITORS.contains(&bundle_id) || bundle_id.starts_with("com.jetbrains.") {
        Some(AppKind::Editor)
    } else {
        None
    }
}

// --- State ------------------------------------------------------------

#[derive(Default)]
struct State {
    /// The project for the dictation in progress (or just finished).
    current: Option<Arc<Project>>,
    /// The last project seen, kept for the Settings line.
    last: Option<(String, usize)>,
    /// Built indexes by repo root.
    indexes: HashMap<PathBuf, (Arc<Project>, Instant)>,
    /// Repo roots with a build running.
    building: Vec<PathBuf>,
    /// App pid -> where its shell/window was last seen.
    detected: HashMap<i32, (Instant, Option<PathBuf>)>,
    /// Repos seen from terminals, to match editor window titles against.
    known_roots: Vec<PathBuf>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

/// Bumped by every `begin`, so a slow build for an earlier dictation can't
/// install itself over a later one.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The project for the current dictation, if one was found and indexed in
/// time.
pub fn current() -> Option<Arc<Project>> {
    state().lock().unwrap().current.clone()
}

/// Called when recording starts, with the frontmost app. Does its work on a
/// background thread: clears the previous project, then (for a terminal or
/// editor, with the setting on) finds the new one and gives its top names
/// to Whisper.
pub fn begin(app: &AppHandle, bundle_id: Option<String>, pid: Option<i32>) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    state().lock().unwrap().current = None;
    set_whisper_terms(app, &[]);

    let (Some(bundle_id), Some(pid)) = (bundle_id, pid) else {
        return;
    };
    let Some(kind) = classify(&bundle_id) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        if !crate::config::load(&app).project_names_enabled {
            return;
        }
        let started = Instant::now();
        let Some(root) = detect_root(kind, &bundle_id, pid) else {
            crate::applog!("project: no repo found for the frontmost {kind:?}");
            return;
        };
        let Some(project) = project_for(&root) else {
            return;
        };
        if GENERATION.load(Ordering::SeqCst) != generation {
            return; // a newer dictation started; don't override its state
        }
        crate::applog!(
            "project: {} names ready ({kind:?}, {}ms)",
            project.index.len(),
            started.elapsed().as_millis()
        );
        {
            let mut st = state().lock().unwrap();
            st.current = Some(project.clone());
            st.last = Some((project.name.clone(), project.index.len()));
        }
        set_whisper_terms(&app, &project.terms);
    });
}

fn set_whisper_terms(app: &AppHandle, terms: &[String]) {
    if let Some(state) = app.try_state::<crate::recording::RecordingState>() {
        state.whisper.set_project_terms(terms.to_vec());
    }
}

/// The index for `root`: the cached one (refreshing it in the background
/// when old), or a fresh build.
fn project_for(root: &Path) -> Option<Arc<Project>> {
    let cached = state().lock().unwrap().indexes.get(root).cloned();
    match cached {
        Some((project, built)) => {
            if built.elapsed() > INDEX_TTL {
                refresh_in_background(root.to_path_buf());
            }
            Some(project)
        }
        None => build_and_store(root),
    }
}

fn refresh_in_background(root: PathBuf) {
    {
        let mut st = state().lock().unwrap();
        if st.building.contains(&root) {
            return;
        }
        st.building.push(root.clone());
    }
    std::thread::spawn(move || {
        build_and_store(&root);
        state().lock().unwrap().building.retain(|r| r != &root);
    });
}

fn build_and_store(root: &Path) -> Option<Arc<Project>> {
    let started = Instant::now();
    let entries = collect(root);
    if entries.is_empty() {
        return None;
    }
    let index = NameIndex::new(entries);
    let terms = index.top_terms(PROMPT_TERMS);
    let name = root.file_name().map_or_else(
        || "project".to_string(),
        |n| n.to_string_lossy().to_string(),
    );
    crate::applog!(
        "project: indexed {} names in {}ms",
        index.len(),
        started.elapsed().as_millis()
    );
    let project = Arc::new(Project { name, index, terms });
    let mut st = state().lock().unwrap();
    st.indexes
        .insert(root.to_path_buf(), (project.clone(), Instant::now()));
    // Keep only the most recently built repos: each index can hold tens of
    // thousands of names, and every worktree is its own root.
    while st.indexes.len() > MAX_INDEXES {
        let Some(oldest) = st
            .indexes
            .iter()
            .min_by_key(|(_, (_, built))| *built)
            .map(|(root, _)| root.clone())
        else {
            break;
        };
        st.indexes.remove(&oldest);
    }
    Some(project)
}

// --- Finding the repo -------------------------------------------------

fn detect_root(kind: AppKind, bundle_id: &str, pid: i32) -> Option<PathBuf> {
    {
        let st = state().lock().unwrap();
        if let Some((at, root)) = st.detected.get(&pid) {
            if at.elapsed() < DETECT_TTL {
                return root.clone();
            }
        }
    }
    let root = match kind {
        AppKind::Terminal => terminal_root(pid),
        AppKind::Editor => {
            let known = state().lock().unwrap().known_roots.clone();
            editor_root(bundle_id, pid, &known)
        }
    };
    let mut st = state().lock().unwrap();
    if let Some(root) = &root {
        if kind == AppKind::Terminal && !st.known_roots.contains(root) {
            st.known_roots.push(root.clone());
            if st.known_roots.len() > 50 {
                st.known_roots.remove(0);
            }
        }
    }
    st.detected.insert(pid, (Instant::now(), root.clone()));
    root
}

/// Runs a command, giving up (and killing it) after `limit`.
fn run(cmd: &mut Command, limit: Duration) -> Option<Vec<u8>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut buf).ok()?;
        Some(buf)
    });
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok().flatten();
                return if status.success() { out } else { None };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = run(
        Command::new("git")
            .arg("-C")
            .arg(root)
            // A repo's own config could name an fsmonitor command, which
            // `git status` would run.
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0"),
        Duration::from_secs(10),
    )?;
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// A regular file's text, if it's under [`SOURCE_FILE_MAX_BYTES`]. Doesn't
/// follow symlinks, so a tracked link to `/dev/zero` or a FIFO can't hang or
/// exhaust memory, and reads no more than the limit even if the file grows.
fn read_small_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > SOURCE_FILE_MAX_BYTES {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(SOURCE_FILE_MAX_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// The git repo containing `dir`, unless that's the home folder itself.
fn git_root(dir: &Path) -> Option<PathBuf> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(top.trim());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    (top.is_dir() && Some(&top) != home.as_ref()).then_some(top)
}

struct Proc {
    pid: i32,
    ppid: i32,
    tty: String,
    foreground: bool,
}

fn process_table() -> Vec<Proc> {
    let Some(out) = run(
        Command::new("ps").args(["-axo", "pid=,ppid=,tty=,stat="]),
        Duration::from_secs(3),
    ) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            Some(Proc {
                pid: f.next()?.parse().ok()?,
                ppid: f.next()?.parse().ok()?,
                tty: f.next()?.to_string(),
                foreground: f.next()?.contains('+'),
            })
        })
        .collect()
}

fn process_cwd(pid: i32) -> Option<PathBuf> {
    let out = run(
        Command::new("lsof").args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"]),
        Duration::from_secs(3),
    )?;
    String::from_utf8_lossy(&out)
        .lines()
        .find_map(|l| l.strip_prefix('n'))
        .map(PathBuf::from)
}

/// The shell working directory of the terminal's most recently used tab or
/// window: of the shells under the app, the one whose tty was touched last.
fn terminal_root(app_pid: i32) -> Option<PathBuf> {
    let table = process_table();
    let mut under = vec![app_pid];
    let mut i = 0;
    while i < under.len() {
        let parent = under[i];
        under.extend(table.iter().filter(|p| p.ppid == parent).map(|p| p.pid));
        i += 1;
    }
    let mut ttys: HashMap<&str, Vec<&Proc>> = HashMap::new();
    for p in table.iter().filter(|p| under.contains(&p.pid)) {
        if p.tty != "??" && p.tty != "-" {
            ttys.entry(p.tty.as_str()).or_default().push(p);
        }
    }
    let last_touched = |tty: &str| {
        std::fs::metadata(format!("/dev/{tty}")).ok().and_then(|m| {
            let modified = m.modified().ok()?;
            let accessed = m.accessed().ok()?;
            Some(modified.max(accessed))
        })
    };
    let (_, mut procs) = ttys.into_iter().max_by_key(|(tty, _)| last_touched(tty))?;
    // The foreground job first (a coding agent's cwd is the project), then
    // the shell; newest first within each.
    procs.sort_by_key(|p| (std::cmp::Reverse(p.foreground), std::cmp::Reverse(p.pid)));
    procs
        .iter()
        .take(4)
        .filter_map(|p| process_cwd(p.pid))
        .find_map(|cwd| git_root(&cwd))
}

fn editor_root(bundle_id: &str, pid: i32, known: &[PathBuf]) -> Option<PathBuf> {
    let storage = vscode_storage_folders(bundle_id);
    if let Some(title) = frontmost_window_title(pid) {
        let segments = title_segments(&title);
        let mut candidates: Vec<PathBuf> = known.to_vec();
        candidates.extend(storage.folders.iter().cloned());
        candidates.extend(common_project_dirs());
        for segment in &segments {
            if let Some(found) = candidates.iter().find(|c| {
                c.file_name()
                    .is_some_and(|n| n.to_string_lossy() == *segment)
            }) {
                return git_root(found);
            }
        }
    }
    // Without a readable title (Screen Recording isn't granted), VS Code
    // and its forks record which folder their last active window has open.
    storage.last_active.and_then(|dir| git_root(&dir))
}

/// The pieces of a window title, e.g. VS Code's "paste.rs — dev-whisper —
/// Visual Studio Code" or JetBrains' "dev-whisper – paste.rs".
fn title_segments(title: &str) -> Vec<String> {
    title
        .replace(" — ", "\u{1}")
        .replace(" – ", "\u{1}")
        .replace(" - ", "\u{1}")
        .split('\u{1}')
        .map(|s| {
            s.trim()
                .trim_start_matches(['●', '•', '*'])
                .trim()
                .trim_end_matches(" (Workspace)")
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn common_project_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    const PARENTS: &[&str] = &[
        "Developer",
        "dev",
        "code",
        "Code",
        "projects",
        "Projects",
        "src",
        "work",
        "repos",
        "github",
        "GitHub",
        "workspace",
        "Documents/GitHub",
        "Documents/Projects",
        "orca/projects",
    ];
    let mut out = Vec::new();
    for parent in PARENTS {
        if let Ok(rd) = std::fs::read_dir(home.join(parent)) {
            for entry in rd.flatten() {
                if entry.path().join(".git").exists() {
                    out.push(entry.path());
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct EditorStorage {
    last_active: Option<PathBuf>,
    folders: Vec<PathBuf>,
}

/// Folders a VS Code-family editor has open, from its own state file.
fn vscode_storage_folders(bundle_id: &str) -> EditorStorage {
    let dir = match bundle_id {
        "com.microsoft.VSCode" => "Code",
        "com.microsoft.VSCodeInsiders" => "Code - Insiders",
        "com.todesktop.230313mzl4w4u92" => "Cursor",
        "com.exafunction.windsurf" => "Windsurf",
        "com.vscodium" => "VSCodium",
        _ => return EditorStorage::default(),
    };
    let Some(home) = std::env::var_os("HOME") else {
        return EditorStorage::default();
    };
    let path = Path::new(&home)
        .join("Library/Application Support")
        .join(dir)
        .join("User/globalStorage/storage.json");
    std::fs::read_to_string(path)
        .ok()
        .map(|json| parse_vscode_storage(&json))
        .unwrap_or_default()
}

fn parse_vscode_storage(json: &str) -> EditorStorage {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return EditorStorage::default();
    };
    let state = &v["windowsState"];
    let mut folders: Vec<PathBuf> = state["openedWindows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["folder"].as_str())
        .filter_map(uri_to_path)
        .collect();
    let last_active = state["lastActiveWindow"]["folder"]
        .as_str()
        .and_then(uri_to_path);
    folders.extend(last_active.clone());
    EditorStorage {
        last_active,
        folders,
    }
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn frontmost_window_title(pid: i32) -> Option<String> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerPID,
    };

    let windows = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )?;
    // Front to back; the first normal window of this app is the focused one.
    for item in windows.iter() {
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(*item as CFDictionaryRef) };
        let get = |key| dict.find(unsafe { CFString::wrap_under_get_rule(key) });
        let owner = get(unsafe { kCGWindowOwnerPID })
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i32());
        let layer = get(unsafe { kCGWindowLayer })
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i32());
        if owner != Some(pid) || layer != Some(0) {
            continue;
        }
        // Titles of other apps' windows need Screen Recording permission;
        // without it the name is simply absent.
        if let Some(name) = get(unsafe { kCGWindowName })
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
        {
            return Some(name);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn frontmost_window_title(_pid: i32) -> Option<String> {
    None
}

// --- Reading names ----------------------------------------------------

/// Everything worth knowing about a repo, before scoring.
fn collect(root: &Path) -> Vec<NameEntry> {
    let started = Instant::now();
    let tracked: Vec<String> = git(root, &["ls-files", "-z"])
        .map(|s| {
            s.split('\0')
                .filter(|p| !p.is_empty() && !is_excluded_path(p))
                .take(TRACKED_FILE_CAP)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if tracked.is_empty() {
        return Vec::new();
    }
    let uncommitted = git(root, &["status", "--porcelain", "-z"])
        .map(|s| parse_porcelain(&s))
        .unwrap_or_default();
    let recent = git(
        root,
        &["log", "--name-only", "--pretty=format:", "-n", "30", "-z"],
    )
    .map(|s| {
        s.split(['\0', '\n'])
            .map(str::to_string)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty() && b != "HEAD");

    let mut deps = Vec::new();
    for path in tracked.iter().filter(|p| p.matches('/').count() <= 2) {
        let base = path.rsplit('/').next().unwrap_or(path);
        if base != "package.json" && base != "Cargo.toml" {
            continue;
        }
        if let Some(text) = read_small_file(&root.join(path)) {
            deps.extend(if base == "package.json" {
                package_json_deps(&text)
            } else {
                cargo_toml_deps(&text)
            });
        }
        if deps.len() > 500 {
            break;
        }
    }

    let mut entries = build_entries(&tracked, &uncommitted, &recent, branch, &deps);

    // Declarations inside the most relevant source files.
    let scores: HashMap<&str, u32> = entries
        .iter()
        .filter(|e| e.kind == Kind::File)
        .map(|e| (e.text.as_str(), e.score))
        .collect();
    let mut sources: Vec<(&str, u32)> = tracked
        .iter()
        .filter(|p| is_source_file(p))
        .map(|p| {
            (
                p.as_str(),
                scores.get(p.as_str()).copied().unwrap_or(SCORE_TRACKED),
            )
        })
        .collect();
    sources.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
    let mut identifiers = 0;
    for (path, score) in sources.into_iter().take(SOURCE_FILES_SCANNED) {
        if started.elapsed() > SCAN_BUDGET || identifiers >= IDENTIFIER_CAP {
            break;
        }
        let Some(text) = read_small_file(&root.join(path)) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for line in text.lines() {
            if let Some(name) = declared_name(line) {
                if seen.insert(name) {
                    entries.push(NameEntry {
                        text: name.to_string(),
                        kind: Kind::Identifier,
                        score,
                    });
                    identifiers += 1;
                }
            }
        }
    }
    entries
}

/// Scores the file paths, branch and packages. Pure, so it's tested.
fn build_entries(
    tracked: &[String],
    uncommitted: &[String],
    recent_commits: &[String],
    branch: Option<String>,
    deps: &[String],
) -> Vec<NameEntry> {
    let mut score: HashMap<&str, u32> = HashMap::new();
    for path in tracked {
        score.insert(path, SCORE_TRACKED);
    }
    // Newest commit first: spread over the recent tier by order of appearance.
    let mut rank = 0u32;
    for path in recent_commits.iter().filter(|p| !p.is_empty()) {
        if let Some(s) = score.get_mut(path.as_str()) {
            if *s == SCORE_TRACKED {
                *s = SCORE_RECENT_COMMIT - (rank / 4).min(SCORE_RECENT_COMMIT - SCORE_BRANCH - 1);
                rank += 1;
            }
        }
    }
    for path in uncommitted {
        if let Some(s) = score.get_mut(path.as_str()) {
            *s = SCORE_UNCOMMITTED;
        }
    }
    let mut entries: Vec<NameEntry> = tracked
        .iter()
        .map(|p| NameEntry {
            text: p.clone(),
            kind: Kind::File,
            score: score[p.as_str()],
        })
        .collect();
    entries.extend(deps.iter().map(|d| NameEntry {
        text: d.clone(),
        kind: Kind::Dependency,
        score: SCORE_DEPENDENCY,
    }));
    if let Some(branch) = branch {
        entries.push(NameEntry {
            text: branch,
            kind: Kind::Branch,
            score: SCORE_BRANCH,
        });
    }
    entries
}

/// Paths from `git status --porcelain -z` (renames list the new path first,
/// then the old one as its own entry).
fn parse_porcelain(out: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut parts = out.split('\0');
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let (status, path) = entry.split_at(3);
        paths.push(path.to_string());
        if status.contains('R') || status.contains('C') {
            parts.next(); // the original path
        }
    }
    paths
}

fn is_excluded_path(path: &str) -> bool {
    const DIRS: &[&str] = &[
        "node_modules",
        "target",
        "vendor",
        "vendored",
        "third_party",
        "dist",
        "build",
        "Pods",
        ".git",
        ".next",
        "__pycache__",
        ".venv",
        "venv",
        "coverage",
    ];
    const BINARY_EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "ico", "icns", "webp", "svg", "woff", "woff2", "ttf", "otf",
        "pdf", "zip", "gz", "tar", "wasm", "bin", "mp3", "mp4", "mov", "wav", "so", "dylib", "a",
        "o", "lock", "map", "onnx", "gguf", "db", "sqlite",
    ];
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            if DIRS.contains(&part) {
                return true;
            }
        } else {
            // Real secrets live in .env files; their names shouldn't even be listed.
            if part.starts_with(".env") || part.contains(".min.") {
                return true;
            }
            if let Some((_, ext)) = part.rsplit_once('.') {
                if BINARY_EXTS.contains(&ext.to_lowercase().as_str()) {
                    return true;
                }
            }
        }
    }
    false
}

fn is_source_file(path: &str) -> bool {
    const EXTS: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "mjs", "py", "go", "java", "kt", "swift", "rb", "c", "h",
        "cpp", "hpp", "cs", "php", "scala", "ex", "exs", "lua", "dart", "zig", "vue", "svelte",
    ];
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| EXTS.contains(&ext))
}

/// The name a source line declares (`pub fn paste_text(`, `export const
/// useAuthStore =`, `class Foo:`), if it's a declaration near the left
/// margin. Cheap and approximate by design.
fn declared_name(line: &str) -> Option<&str> {
    const MODIFIERS: &[&str] = &[
        "pub",
        "export",
        "default",
        "async",
        "unsafe",
        "extern",
        "abstract",
        "public",
        "private",
        "protected",
        "static",
        "final",
        "declare",
        "open",
        "internal",
        "override",
        "sealed",
        "data",
    ];
    const KEYWORDS: &[&str] = &[
        "fn",
        "struct",
        "enum",
        "trait",
        "type",
        "function",
        "class",
        "interface",
        "def",
        "func",
        "mod",
        "const",
        "object",
        "protocol",
        "record",
    ];
    const GENERIC: &[&str] = &[
        "new", "main", "test", "tests", "default", "from", "into", "drop", "fmt", "get", "set",
        "run", "init", "setup", "index", "Props", "State", "Error", "Result", "self", "impl",
    ];
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 4 {
        return None;
    }
    let mut tokens = trimmed.split_whitespace();
    let mut exported = false;
    let keyword = loop {
        let token = tokens.next()?;
        let base = token.split('(').next().unwrap_or(token);
        if MODIFIERS.contains(&base) {
            exported |= matches!(base, "pub" | "export" | "public");
        } else if KEYWORDS.contains(&token) {
            break token;
        } else {
            return None;
        }
    };
    if keyword == "const" && !exported {
        return None;
    }
    let mut next = tokens.next()?;
    if keyword == "const" && next == "fn" {
        next = tokens.next()?;
    }
    if keyword == "func" && next.starts_with('(') {
        // Go method receiver: skip to the token that closes it.
        while !next.contains(')') {
            next = tokens.next()?;
        }
        next = tokens.next()?;
    }
    let end = next
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(next.len());
    let name = &next[..end];
    let valid = name.chars().count() >= 3
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && !GENERIC.contains(&name);
    valid.then_some(name)
}

fn package_json_deps(text: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    ["dependencies", "devDependencies"]
        .iter()
        .filter_map(|k| v[k].as_object())
        .flat_map(|o| o.keys().cloned())
        .collect()
}

fn cargo_toml_deps(text: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.trim_matches(['[', ']']).ends_with("dependencies");
        } else if in_deps {
            if let Some((name, _)) = line.split_once('=') {
                let name = name.trim().split('.').next().unwrap_or("").trim();
                if !name.is_empty() && !name.starts_with('#') {
                    deps.push(name.to_string());
                }
            }
        }
    }
    deps
}

// --- Settings commands --------------------------------------------------

#[tauri::command]
pub fn get_project_names_enabled(app: AppHandle) -> bool {
    crate::config::load(&app).project_names_enabled
}

#[tauri::command]
pub fn set_project_names_enabled(app: AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.project_names_enabled = enabled;
    let _ = crate::config::save(&app, &cfg);
    if !enabled {
        state().lock().unwrap().current = None;
        set_whisper_terms(&app, &[]);
    }
}

#[derive(serde::Serialize)]
pub struct ProjectStatus {
    pub name: String,
    pub names: usize,
}

/// The last project Mouthpiece found names for, for the Settings line.
#[tauri::command]
pub fn get_project_status() -> Option<ProjectStatus> {
    state()
        .lock()
        .unwrap()
        .last
        .clone()
        .map(|(name, names)| ProjectStatus { name, names })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_terminals_and_editors_only() {
        assert_eq!(classify("com.apple.Terminal"), Some(AppKind::Terminal));
        assert_eq!(classify("com.googlecode.iterm2"), Some(AppKind::Terminal));
        assert_eq!(classify("com.microsoft.VSCode"), Some(AppKind::Editor));
        assert_eq!(classify("com.jetbrains.intellij"), Some(AppKind::Editor));
        assert_eq!(classify("com.tinyspeck.slackmacgap"), None);
        assert_eq!(classify("com.google.Chrome"), None);
    }

    #[test]
    fn declarations_are_found_across_languages() {
        let cases = [
            (
                "pub fn paste_text(text: &str) -> Result<(), String> {",
                Some("paste_text"),
            ),
            ("    pub(crate) async fn load_all() {", Some("load_all")),
            ("pub struct WhisperEngine {", Some("WhisperEngine")),
            (
                "pub const MAX_RETRY_COUNT: usize = 3;",
                Some("MAX_RETRY_COUNT"),
            ),
            ("const LOCAL_ONLY: usize = 3;", None),
            (
                "export const useAuthStore = create<State>()(",
                Some("useAuthStore"),
            ),
            (
                "export default function SettingsPage() {",
                Some("SettingsPage"),
            ),
            ("export interface AppConfig {", Some("AppConfig")),
            ("class UserRepository:", Some("UserRepository")),
            ("    def handle_request(self, req):", Some("handle_request")),
            (
                "func (s *Server) HandleRequest(w http.ResponseWriter) {",
                Some("HandleRequest"),
            ),
            ("        fn too_deep() {}", None),
            ("pub fn new() -> Self {", None),
            ("// pub fn commented_out() {", None),
            ("let x = 5;", None),
        ];
        for (line, want) in cases {
            assert_eq!(declared_name(line), want, "{line}");
        }
    }

    #[test]
    fn env_files_binaries_and_vendored_dirs_are_skipped() {
        for skipped in [
            ".env",
            "app/.env.local",
            ".env.example",
            "node_modules/x/index.js",
            "src-tauri/target/debug/x.rs",
            "assets/logo.png",
            "Cargo.lock",
            "dist/app.min.js",
            "vendor/lib/a.go",
        ] {
            assert!(is_excluded_path(skipped), "{skipped}");
        }
        for kept in [
            "src/lib.rs",
            "src/target.rs",
            "README.md",
            "e2e/app.spec.ts",
        ] {
            assert!(!is_excluded_path(kept), "{kept}");
        }
    }

    #[test]
    fn porcelain_paths_including_renames() {
        let out = " M src/a.rs\0?? new file.txt\0R  src/new.rs\0src/old.rs\0A  b.rs\0";
        assert_eq!(
            parse_porcelain(out),
            ["src/a.rs", "new file.txt", "src/new.rs", "b.rs"]
        );
    }

    #[test]
    fn entries_rank_uncommitted_over_recent_over_tracked() {
        let tracked: Vec<String> = ["a.rs", "b.rs", "c.rs"].map(String::from).to_vec();
        let entries = build_entries(
            &tracked,
            &["c.rs".to_string()],
            &["b.rs".to_string(), "c.rs".to_string(), String::new()],
            Some("feature/names".into()),
            &["serde".to_string()],
        );
        let score = |t: &str| entries.iter().find(|e| e.text == t).unwrap().score;
        assert_eq!(score("c.rs"), SCORE_UNCOMMITTED);
        assert!(score("b.rs") >= SCORE_BRANCH && score("b.rs") <= SCORE_RECENT_COMMIT);
        assert_eq!(score("a.rs"), SCORE_TRACKED);
        assert_eq!(score("serde"), SCORE_DEPENDENCY);
        assert_eq!(score("feature/names"), SCORE_BRANCH);
    }

    #[test]
    fn dependency_names_come_from_both_manifests() {
        assert_eq!(
            package_json_deps(r#"{"dependencies":{"react":"1"},"devDependencies":{"vite":"1"}}"#),
            ["react", "vite"]
        );
        let toml = "[package]\nname = \"x\"\n[dependencies]\nserde = \"1\"\ntauri = { version = \"2\" }\n[dev-dependencies]\ntempfile = \"3\"\n[profile.release]\nlto = true\n";
        assert_eq!(cargo_toml_deps(toml), ["serde", "tauri", "tempfile"]);
    }

    #[test]
    fn editor_titles_and_state_files_parse() {
        assert_eq!(
            title_segments("paste.rs — dev-whisper — Visual Studio Code"),
            ["paste.rs", "dev-whisper", "Visual Studio Code"]
        );
        assert_eq!(
            title_segments("● app.tsx - my app (Workspace)"),
            ["app.tsx", "my app"]
        );
        assert_eq!(
            uri_to_path("file:///Users/me/My%20Proj"),
            Some(PathBuf::from("/Users/me/My Proj"))
        );
        let storage = parse_vscode_storage(
            r#"{"windowsState":{"lastActiveWindow":{"folder":"file:///a/b"},"openedWindows":[{"folder":"file:///c/d"}]}}"#,
        );
        assert_eq!(storage.last_active, Some(PathBuf::from("/a/b")));
        assert_eq!(storage.folders.len(), 2);
    }

    /// Reads this very repo, if git is around: finds names and applies them.
    #[test]
    fn indexes_a_real_repo() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let Some(top) = git_root(root) else {
            return;
        };
        let entries = collect(&top);
        assert!(entries
            .iter()
            .any(|e| e.text.ends_with("src-tauri/src/paste.rs")));
        assert!(entries.iter().any(|e| e.text == "paste_text"));
        assert!(entries.iter().all(|e| !e.text.contains(".env")));
        let index = NameIndex::new(entries);
        let (out, n) = index.apply("fix the paste text function in the paste file", true);
        assert_eq!(n, 2, "{out}");
        assert_eq!(
            out,
            "fix the paste_text function in the @src-tauri/src/paste.rs"
        );
    }
}
