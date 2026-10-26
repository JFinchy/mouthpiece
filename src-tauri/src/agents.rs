//! Replying to a waiting coding agent (Claude Code in a terminal pane)
//! without switching windows.
//!
//! Claude Code's Notification and Stop hooks run `mouthpiece-hook`, which
//! opens `mouthpiece://agent-waiting?...`. Mouthpiece works out which app
//! owns the terminal the agent runs in (by walking the hook's parent
//! processes), keeps a short queue of waiting agents, and shows a pill in the
//! widget. The reply shortcut records a dictation, then hops to the agent's
//! window, pastes, and hops back.

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::config::{self, ShortcutConfig};

/// A waiting agent is forgotten after this long without an update.
const EXPIRY_MS: u64 = 30 * 60 * 1000;
const MAX_WAITING: usize = 8;
const MAX_MESSAGE_CHARS: usize = 160;
/// Longest wait for the agent's app to become frontmost before giving up.
const ACTIVATE_POLLS: u32 = 24;
const ACTIVATE_POLL_EVERY: Duration = Duration::from_millis(25);
/// Lets the raised window take keyboard focus before Cmd+V is sent.
const SETTLE_AFTER_RAISE: Duration = Duration::from_millis(90);
/// Lets the target app read the pasted text before focus moves back.
const SETTLE_AFTER_PASTE: Duration = Duration::from_millis(220);

const HOOK_SCRIPT: &str = include_str!("../resources/mouthpiece-hook");
const TOKEN_PLACEHOLDER: &str = "__DEV_WHISPER_TOKEN__";
const SOUND_PATH: &str = "/System/Library/Sounds/Tink.aiff";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Permission prompt or other question.
    NeedsInput,
    /// The agent finished its turn.
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WaitingAgent {
    pub session_id: String,
    pub cwd: String,
    /// Last path component of `cwd`: what the widget shows.
    pub folder: String,
    pub message: String,
    pub kind: Kind,
    pub app_pid: Option<i32>,
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub tty: Option<String>,
    pub received_ms: u64,
}

// ---------------------------------------------------------------- deep link

/// What the hook script put in the link, before the owning app is resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub session_id: String,
    pub cwd: String,
    pub message: String,
    pub kind: Kind,
    /// The hook's parent process.
    pub pid: Option<i32>,
    /// `__CFBundleIdentifier` of the hook's environment.
    pub bundle: Option<String>,
    pub term_program: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkEvent {
    Waiting(Report),
    /// The user answered in the terminal themselves.
    Resumed {
        session_id: String,
    },
}

fn folder_name(cwd: &str) -> String {
    cwd.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("agent")
        .to_string()
}

fn clean_message(raw: &str) -> String {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_MESSAGE_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_MESSAGE_CHARS - 1).collect();
    format!("{cut}…")
}

/// Sessions are told apart by id; a hook without one falls back to the
/// folder, so two events from the same folder still collapse into one.
fn session_key(session: &str, cwd: &str) -> String {
    if session.is_empty() {
        format!("cwd:{cwd}")
    } else {
        session.to_string()
    }
}

pub fn parse_link(url: &tauri::Url) -> Result<LinkEvent, String> {
    let pairs: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    let get = |k: &str| pairs.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let session = get("session").unwrap_or_default();
    let cwd = get("cwd").unwrap_or_default();
    if session.is_empty() && cwd.is_empty() {
        return Err("needs a session or cwd".to_string());
    }
    let key = session_key(session, cwd);

    match url.host_str().unwrap_or("") {
        "agent-resumed" => Ok(LinkEvent::Resumed { session_id: key }),
        "agent-waiting" => {
            let kind = match get("kind") {
                Some("done") => Kind::Done,
                _ => Kind::NeedsInput,
            };
            let message = clean_message(get("message").unwrap_or_default());
            Ok(LinkEvent::Waiting(Report {
                session_id: key,
                cwd: cwd.to_string(),
                message,
                kind,
                pid: get("pid").and_then(|p| p.parse().ok()).filter(|p| *p > 1),
                bundle: get("bundle").map(str::to_string),
                term_program: get("term").map(str::to_string),
            }))
        }
        other => Err(format!("unrecognized agent action {other:?}")),
    }
}

// -------------------------------------------------------------------- queue

#[derive(Default)]
pub struct AgentQueue {
    /// Newest first.
    items: Vec<WaitingAgent>,
}

impl AgentQueue {
    /// Adds or refreshes an agent (one entry per session, moved to the
    /// front). Returns true when the user should be told: a new session, or
    /// the same one with a different kind or message.
    pub fn upsert(&mut self, agent: WaitingAgent, now_ms: u64) -> bool {
        self.prune(now_ms);
        let changed = match self
            .items
            .iter()
            .position(|a| a.session_id == agent.session_id)
        {
            Some(i) => {
                let old = self.items.remove(i);
                old.kind != agent.kind || old.message != agent.message
            }
            None => true,
        };
        self.items.insert(0, agent);
        self.items.truncate(MAX_WAITING);
        changed
    }

    pub fn remove(&mut self, session_id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|a| a.session_id != session_id);
        self.items.len() != before
    }

    pub fn prune(&mut self, now_ms: u64) {
        self.items
            .retain(|a| now_ms.saturating_sub(a.received_ms) < EXPIRY_MS);
    }

    pub fn list(&mut self, now_ms: u64) -> Vec<WaitingAgent> {
        self.prune(now_ms);
        self.items.clone()
    }

    pub fn latest(&mut self, now_ms: u64) -> Option<WaitingAgent> {
        self.prune(now_ms);
        self.items.first().cloned()
    }

    pub fn get(&mut self, session_id: &str, now_ms: u64) -> Option<WaitingAgent> {
        self.prune(now_ms);
        self.items
            .iter()
            .find(|a| a.session_id == session_id)
            .cloned()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// --------------------------------------------------------- owner resolution

pub struct ProcInfo {
    pub ppid: i32,
    /// Controlling terminal, e.g. "ttys003"; `None` for none.
    pub tty: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Owner {
    pub pid: i32,
    pub bundle_id: String,
    pub name: String,
    pub tty: Option<String>,
}

fn bundle_for_term_program(term: &str) -> Option<&'static str> {
    Some(match term {
        "Apple_Terminal" => "com.apple.Terminal",
        "iTerm.app" => "com.googlecode.iterm2",
        "WarpTerminal" => "dev.warp.Warp-Stable",
        "WezTerm" => "com.github.wez.wezterm",
        "ghostty" => "com.mitchellh.ghostty",
        "vscode" => "com.microsoft.VSCode",
        _ => return None,
    })
}

/// Finds the app that owns the terminal the hook ran in. First choice is
/// walking up from the hook's parent process to the first regular app
/// (exact: it is the process that actually spawned the terminal). Without
/// that, the bundle id macOS exports to child processes, then
/// `TERM_PROGRAM`, which is a guess (Cursor also says "vscode").
pub fn resolve_owner(
    report: &Report,
    proc_info: &dyn Fn(i32) -> Option<ProcInfo>,
    app_for_pid: &dyn Fn(i32) -> Option<(String, String)>,
    app_for_bundle: &dyn Fn(&str) -> Option<(i32, String)>,
) -> Option<Owner> {
    let mut tty = None;
    let mut seen = Vec::new();
    let mut pid = report.pid;
    while let Some(current) = pid {
        if current <= 1 || seen.contains(&current) || seen.len() > 40 {
            break;
        }
        seen.push(current);
        if let Some((bundle_id, name)) = app_for_pid(current) {
            return Some(Owner {
                pid: current,
                bundle_id,
                name,
                tty,
            });
        }
        let Some(info) = proc_info(current) else {
            break;
        };
        if tty.is_none() {
            tty = info.tty;
        }
        pid = Some(info.ppid);
    }

    let bundle = report.bundle.as_deref().or_else(|| {
        report
            .term_program
            .as_deref()
            .and_then(bundle_for_term_program)
    })?;
    let (pid, name) = app_for_bundle(bundle)?;
    Some(Owner {
        pid,
        bundle_id: bundle.to_string(),
        name,
        tty,
    })
}

fn ps_info(pid: i32) -> Option<ProcInfo> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "ppid=,tty=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

fn parse_ps(output: &str) -> Option<ProcInfo> {
    let mut parts = output.split_whitespace();
    let ppid = parts.next()?.parse().ok()?;
    let tty = parts
        .next()
        .filter(|t| !t.starts_with('?') && *t != "-")
        .map(str::to_string);
    Some(ProcInfo { ppid, tty })
}

// ------------------------------------------------------- window selection

#[derive(Debug, PartialEq)]
pub enum WindowChoice {
    /// A window title names the agent's terminal device, so the tab or pane
    /// in front is the agent's own.
    Tty(usize),
    /// A window title names the agent's folder. The selected tab or pane in
    /// it could still be a different shell in the same folder.
    Match(usize),
    /// Nothing names it, but the app has only one window.
    Only(usize),
}

/// Picks which of an app's windows the agent is in. Refuses to guess
/// between several windows, so a reply never lands in the wrong one.
pub fn choose_window(
    titles: &[String],
    folder: &str,
    tty: Option<&str>,
) -> Result<WindowChoice, String> {
    let folder = folder.to_lowercase();
    let tty = tty.map(str::to_lowercase);
    let hits = |needle: &str| -> Vec<usize> {
        titles
            .iter()
            .enumerate()
            .filter(|(_, t)| !needle.is_empty() && t.to_lowercase().contains(needle))
            .map(|(i, _)| i)
            .collect()
    };

    if let Some(tty) = &tty {
        if let [only] = hits(tty)[..] {
            return Ok(WindowChoice::Tty(only));
        }
    }
    match hits(&folder)[..] {
        [only] => return Ok(WindowChoice::Match(only)),
        [_, _, ..] => return Err(format!("more than one window mentions {folder}")),
        [] => {}
    }
    match titles.len() {
        0 => Err("it has no open windows".to_string()),
        1 => Ok(WindowChoice::Only(0)),
        n => Err(format!("none of its {n} windows mention {folder}")),
    }
}

/// In a reply, "send it" (or "send that") after a clause break means the
/// same as "press enter".
pub fn normalize_reply(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches(['.', '!', '?']).trim_end();
    for phrase in ["send it", "send that"] {
        let Some(prefix) = crate::clipboard::strip_suffix_ignore_case(trimmed, phrase) else {
            continue;
        };
        // Same rule as `extract_press_enter`: only after a clause break, so
        // "fix the bug and send it" stays a sentence.
        let head = prefix.trim_end();
        if head.is_empty() || crate::punctuation::ends_clause(head) {
            return format!("{head} press enter").trim().to_string();
        }
    }
    text.to_string()
}

// -------------------------------------------------------------------- state

pub struct AgentState {
    /// Secret written into the installed hook script. Agent links must
    /// carry it, because any web page can open a `mouthpiece://` link.
    token: Option<String>,
    queue: Mutex<AgentQueue>,
    /// The agent the dictation now being recorded is for.
    pending_reply: Mutex<Option<WaitingAgent>>,
    shortcut: Mutex<ShortcutConfig>,
    shortcut_error: Mutex<Option<String>>,
}

pub fn default_reply_shortcut() -> ShortcutConfig {
    ShortcutConfig {
        meta: false,
        ctrl: true,
        alt: true,
        shift: false,
        code: "KeyR".to_string(),
    }
}

impl AgentState {
    /// The agent the recording in progress is for, left in place.
    pub fn pending_reply(&self) -> Option<WaitingAgent> {
        self.pending_reply.lock().unwrap().clone()
    }

    pub fn take_pending_reply(&self) -> Option<WaitingAgent> {
        self.pending_reply.lock().unwrap().take()
    }

    pub fn is_reply_shortcut(&self, triggered: &tauri_plugin_global_shortcut::Shortcut) -> bool {
        crate::shortcut::to_shortcut(&self.shortcut.lock().unwrap()).is_ok_and(|s| s == *triggered)
    }
}

/// Registers the reply shortcut and writes the hook script; called once at
/// launch. A shortcut another app owns is logged, not fatal.
pub fn init(app: &AppHandle, cfg: &config::AppConfig) -> AgentState {
    let shortcut = cfg
        .agent_reply_shortcut
        .clone()
        .unwrap_or_else(default_reply_shortcut);
    let error = register(app, &shortcut).err();
    if let Some(err) = &error {
        crate::applog!("agents: could not register reply shortcut {shortcut:?}: {err}");
    }
    let token = hook_token(app)
        .map_err(|err| crate::applog!("agents: could not set up the hook token: {err}"))
        .ok();
    if let Some(token) = &token {
        if let Err(err) = install_hook_script(app, token) {
            crate::applog!("agents: could not install the hook script: {err}");
        }
    }
    AgentState {
        token,
        queue: Default::default(),
        pending_reply: Default::default(),
        shortcut: Mutex::new(shortcut),
        shortcut_error: Mutex::new(error),
    }
}

fn register(app: &AppHandle, cfg: &ShortcutConfig) -> Result<(), String> {
    let shortcut = crate::shortcut::to_shortcut(cfg)?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())
}

fn hook_script_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("bin").join("mouthpiece-hook"))
}

/// This install's hook token: read from the app data folder, or created
/// there (readable only by the user) on first launch.
fn hook_token(app: &AppHandle) -> Result<String, String> {
    use std::io::Read;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let path = dir.join("agent-hook-token");
    if let Ok(saved) = std::fs::read_to_string(&path) {
        let saved = saved.trim();
        if saved.len() >= 32 && saved.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(saved.to_string());
        }
    }
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, token.as_bytes()))
        .map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Ok(token)
}

/// The hook script with this install's token filled in.
pub fn hook_script_with_token(token: &str) -> String {
    HOOK_SCRIPT.replace(TOKEN_PLACEHOLDER, token)
}

/// Whether a link carries this install's token.
pub fn link_has_token(url: &tauri::Url, token: &str) -> bool {
    url.query_pairs()
        .any(|(k, v)| k == "token" && v.as_bytes() == token.as_bytes())
}

/// Keeps a copy of the hook script at a stable path outside the app bundle,
/// so the snippet in `~/.claude/settings.json` survives app updates and
/// works from a dev build too. Only the user can read it, since it holds
/// the token.
fn install_hook_script(app: &AppHandle, token: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let path = hook_script_path(app)?;
    let script = hook_script_with_token(token);
    let dir = path.parent().ok_or("no folder for the hook script")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    // Hook entries added before the rename run `dev-whisper-hook`; keep
    // that copy current too (see rename.rs).
    let legacy = dir.join("dev-whisper-hook");
    for path in std::iter::once(&path).chain(legacy.exists().then_some(&legacy)) {
        if std::fs::read_to_string(path).ok().as_deref() != Some(script.as_str()) {
            std::fs::write(path, &script).map_err(|e| e.to_string())?;
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The block to merge into `~/.claude/settings.json`.
pub fn hook_config_json(script_path: &str) -> String {
    let command = format!("'{}'", script_path.replace('\'', r"'\''"));
    let entry = serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]);
    let config = serde_json::json!({
        "hooks": {
            "Notification": entry,
            "Stop": entry,
            "UserPromptSubmit": entry,
        }
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

// -------------------------------------------------------------- deep links

fn emit_changed(app: &AppHandle) {
    let state = app.state::<AgentState>();
    let list = state.queue.lock().unwrap().list(now_ms());
    let _ = app.emit("agents-changed", list);
}

/// Handles `agent-waiting` and `agent-resumed` links. Resolving the owning
/// app shells out to `ps`, so it runs off the deep-link callback.
pub fn handle_link(app: &AppHandle, url: &tauri::Url) {
    let token = app.state::<AgentState>().token.clone();
    if !token.is_some_and(|token| link_has_token(url, &token)) {
        crate::applog!("agents: ignoring a link without this install's token");
        return;
    }
    let event = match parse_link(url) {
        Ok(event) => event,
        Err(err) => {
            crate::applog!("agents: ignoring link: {err}");
            return;
        }
    };
    let app = app.clone();
    std::thread::spawn(move || match event {
        LinkEvent::Resumed { session_id } => {
            let removed = app
                .state::<AgentState>()
                .queue
                .lock()
                .unwrap()
                .remove(&session_id);
            if removed {
                crate::applog!("agents: {session_id} resumed");
                emit_changed(&app);
            }
        }
        LinkEvent::Waiting(report) => {
            let owner = resolve_owner(
                &report,
                &ps_info,
                &crate::app_detect::regular_app_for_pid,
                &crate::app_detect::regular_app_for_bundle,
            );
            let folder = folder_name(&report.cwd);
            crate::applog!(
                "agents: waiting session={} folder={folder} kind={:?} app={:?}",
                report.session_id,
                report.kind,
                owner.as_ref().map(|o| (&o.name, o.pid))
            );
            let agent = WaitingAgent {
                session_id: report.session_id,
                folder,
                cwd: report.cwd,
                message: report.message,
                kind: report.kind,
                app_pid: owner.as_ref().map(|o| o.pid),
                bundle_id: owner.as_ref().map(|o| o.bundle_id.clone()),
                app_name: owner.as_ref().map(|o| o.name.clone()),
                tty: owner.and_then(|o| o.tty),
                received_ms: now_ms(),
            };
            let notify = app
                .state::<AgentState>()
                .queue
                .lock()
                .unwrap()
                .upsert(agent, now_ms());
            emit_changed(&app);
            if notify && crate::config::load(&app).agent_sound_enabled {
                play_sound();
            }
        }
    });
}

fn play_sound() {
    std::thread::spawn(|| {
        let _ = std::process::Command::new("/usr/bin/afplay")
            .arg(SOUND_PATH)
            .status();
    });
}

// ------------------------------------------------------------ reply hotkey

/// The reply shortcut: starts a dictation addressed to the newest waiting
/// agent, or stops the one in progress.
pub fn reply_hotkey(app: &AppHandle) {
    use crate::capture::{CaptureState, Request, Trigger};
    let recording = app.state::<crate::recording::RecordingState>();
    match recording.capture.state() {
        CaptureState::Dictating { .. } => {
            crate::recording::request_dictation(app, Request::StopDictation(Trigger::Hotkey));
        }
        CaptureState::Enrolling { .. } => {}
        CaptureState::Idle => {
            let state = app.state::<AgentState>();
            let Some(target) = state.queue.lock().unwrap().latest(now_ms()) else {
                crate::applog!("agents: reply shortcut pressed with no agent waiting");
                let _ = app.emit(
                    "transcript-error",
                    "No coding agent is waiting. Set up the hook in Settings > Coding agents."
                        .to_string(),
                );
                return;
            };
            crate::applog!(
                "agents: recording a reply for {} ({:?})",
                target.folder,
                target.app_name
            );
            *state.pending_reply.lock().unwrap() = Some(target);
            crate::recording::request_dictation(app, Request::StartDictation(Trigger::Hotkey));
            if !matches!(recording.capture.state(), CaptureState::Dictating { .. }) {
                state.take_pending_reply();
            }
        }
    }
}

pub struct ReplyOutcome {
    pub result: Result<(), String>,
    /// Shown in the widget.
    pub notice: String,
    /// The text went to the clipboard instead of the agent.
    pub copied: bool,
}

fn copied(text: &str, why: String) -> ReplyOutcome {
    ReplyOutcome {
        result: if text.is_empty() {
            Ok(())
        } else {
            crate::paste::copy_text(text)
        },
        notice: format!("Copied. {why}"),
        copied: true,
    }
}

/// Hops to the agent's window, pastes (and presses Enter if asked), and
/// hops back. Falls back to the clipboard whenever it can't be sure the
/// text will land in the agent's app.
///
/// Enter is only pressed when the window title names the agent's own
/// terminal device. Otherwise the selected tab could be a plain shell in the
/// same folder, where Enter would run the reply as a command, so the text
/// is pasted and focus stays there for the user to check and press Return.
pub fn deliver_reply(
    app: &AppHandle,
    target: &WaitingAgent,
    text: &str,
    press_enter: bool,
    copy_only: bool,
) -> ReplyOutcome {
    let started = Instant::now();
    let app_label = target.app_name.clone().unwrap_or_else(|| "its app".into());
    if text.is_empty() && !press_enter {
        return ReplyOutcome {
            result: Ok(()),
            notice: "Nothing to send".to_string(),
            copied: true,
        };
    }
    if copy_only {
        return copied(text, "Copy only is on, so nothing was sent.".into());
    }
    let Some(pid) = target.app_pid else {
        return copied(
            text,
            format!("Couldn't tell which app {} runs in.", target.folder),
        );
    };
    // Where to return to afterwards. Skipped when that is Mouthpiece itself
    // (the user had just clicked the widget).
    let previous = crate::app_detect::frontmost_pid_blocking(app)
        .filter(|prev| *prev != std::process::id() as i32)
        .map(crate::focus::snapshot);

    let (folder, tty) = (target.folder.clone(), target.tty.clone());
    let choice = std::cell::Cell::new(None);
    let raised = crate::focus::raise_chosen(pid, &|titles| {
        let picked = choose_window(titles, &folder, tty.as_deref())?;
        let index = match &picked {
            WindowChoice::Tty(i) | WindowChoice::Match(i) | WindowChoice::Only(i) => *i,
        };
        choice.set(Some(picked));
        Ok(index)
    });
    let title = match raised {
        Ok(title) => title,
        Err(why) => {
            if let Some(snapshot) = &previous {
                crate::focus::restore(snapshot);
            }
            crate::applog!("agents: could not raise {app_label}: {why}");
            return copied(text, format!("Couldn't reach {app_label}: {why}."));
        }
    };
    let raise_ms = started.elapsed().as_millis();

    // Never paste unless the agent's app is really in front.
    let mut frontmost = false;
    for _ in 0..ACTIVATE_POLLS {
        if crate::app_detect::frontmost_pid_blocking(app) == Some(pid) {
            frontmost = true;
            break;
        }
        std::thread::sleep(ACTIVATE_POLL_EVERY);
    }
    if !frontmost {
        if let Some(snapshot) = &previous {
            crate::focus::restore(snapshot);
        }
        return copied(text, format!("{app_label} didn't come to the front."));
    }
    std::thread::sleep(SETTLE_AFTER_RAISE);
    let kind = choice.take().unwrap_or(WindowChoice::Only(0));
    if matches!(kind, WindowChoice::Tty(_) | WindowChoice::Match(_))
        && crate::focus::focused_window_title(pid).is_some_and(|now| now != title)
    {
        if let Some(snapshot) = &previous {
            crate::focus::restore(snapshot);
        }
        return copied(
            text,
            format!("The {folder} window didn't come to the front."),
        );
    }

    let delivered = if text.is_empty() {
        Ok(())
    } else {
        crate::paste::paste_text(text)
    };
    let tab_confirmed = matches!(kind, WindowChoice::Tty(_));
    let hold = press_enter && !tab_confirmed;
    let delivered = delivered.and_then(|()| {
        if press_enter && !hold {
            crate::paste::press_enter()
        } else {
            Ok(())
        }
    });
    let pasted_ms = started.elapsed().as_millis();

    // Return to wherever the user was, unless they need to press Return.
    std::thread::sleep(SETTLE_AFTER_PASTE);
    let returned = match &previous {
        Some(snapshot) if !hold => crate::focus::restore(snapshot),
        _ => false,
    };
    crate::applog!(
        "agents: reply hop app={app_label} window={title:?} raise={raise_ms}ms paste={}ms \
         total={}ms returned={returned}",
        pasted_ms - raise_ms,
        started.elapsed().as_millis()
    );

    if delivered.is_ok() && !hold {
        // Answered, so it stops showing as waiting.
        app.state::<AgentState>()
            .queue
            .lock()
            .unwrap()
            .remove(&target.session_id);
        emit_changed(app);
    }

    let notice = match delivered {
        Ok(()) if hold && text.is_empty() => {
            format!("In {app_label}. Check it's the {folder} agent's tab, then press Return.")
        }
        Ok(()) if hold => format!(
            "Pasted in {app_label}. Check it's the {folder} agent's tab, then press Return."
        ),
        Ok(()) if matches!(kind, WindowChoice::Tty(_) | WindowChoice::Match(_)) => {
            format!("Sent to {folder} in {app_label}")
        }
        Ok(()) => format!("Sent to {app_label} (can't pick the exact tab)"),
        Err(_) => String::new(),
    };
    ReplyOutcome {
        result: delivered,
        notice,
        copied: false,
    }
}

// ----------------------------------------------------------------- commands

#[tauri::command]
pub fn get_waiting_agents(state: tauri::State<AgentState>) -> Vec<WaitingAgent> {
    state.queue.lock().unwrap().list(now_ms())
}

#[tauri::command]
pub fn dismiss_waiting_agent(app: AppHandle, session_id: String) {
    app.state::<AgentState>()
        .queue
        .lock()
        .unwrap()
        .remove(&session_id);
    emit_changed(&app);
}

/// Brings a waiting agent's window to the front (the widget pill's click).
/// Unlike a reply, nothing is pasted and focus is not returned.
#[tauri::command(async)]
pub fn focus_waiting_agent(app: AppHandle, session_id: String) -> Result<String, String> {
    let target = app
        .state::<AgentState>()
        .queue
        .lock()
        .unwrap()
        .get(&session_id, now_ms())
        .ok_or("That agent is no longer waiting.")?;
    let app_label = target.app_name.clone().unwrap_or_else(|| "its app".into());
    let pid = target
        .app_pid
        .ok_or_else(|| format!("Couldn't tell which app {} runs in.", target.folder))?;
    crate::focus::raise_chosen(pid, &|titles| match choose_window(
        titles,
        &target.folder,
        target.tty.as_deref(),
    )? {
        WindowChoice::Tty(i) | WindowChoice::Match(i) | WindowChoice::Only(i) => Ok(i),
    })
    .map_err(|why| format!("Couldn't reach {app_label}: {why}."))?;
    crate::applog!("agents: focused {} in {app_label}", target.folder);
    Ok(app_label)
}

#[tauri::command]
pub fn get_agent_reply_shortcut(state: tauri::State<AgentState>) -> ShortcutConfig {
    state.shortcut.lock().unwrap().clone()
}

/// Why the reply shortcut isn't registered, if it isn't.
#[tauri::command]
pub fn get_agent_reply_shortcut_error(state: tauri::State<AgentState>) -> Option<String> {
    state.shortcut_error.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_agent_reply_shortcut(
    app: AppHandle,
    new_cfg: ShortcutConfig,
    state: tauri::State<AgentState>,
    push_to_talk: tauri::State<crate::shortcut::PushToTalkState>,
) -> Result<ShortcutConfig, String> {
    let new_shortcut = crate::shortcut::to_shortcut(&new_cfg)?;
    let main = push_to_talk.current.lock().unwrap().clone();
    let taken = [main.clone(), crate::shortcut::skip_variant(&main)]
        .iter()
        .any(|c| crate::shortcut::to_shortcut(c).is_ok_and(|s| s == new_shortcut));
    if taken {
        return Err("That is the push-to-talk shortcut. Pick a different one.".to_string());
    }

    let mut current = state.shortcut.lock().unwrap();
    let old = crate::shortcut::to_shortcut(&current)
        .ok()
        .filter(|old| app.global_shortcut().is_registered(*old));
    if old != Some(new_shortcut) {
        if let Some(old) = old {
            let _ = app.global_shortcut().unregister(old);
        }
        if let Err(err) = app.global_shortcut().register(new_shortcut) {
            if let Some(old) = old {
                let _ = app.global_shortcut().register(old);
            }
            return Err(err.to_string());
        }
    }
    *current = new_cfg.clone();
    drop(current);
    *state.shortcut_error.lock().unwrap() = None;

    let mut cfg = config::load(&app);
    cfg.agent_reply_shortcut = Some(new_cfg.clone());
    let _ = config::save(&app, &cfg);
    Ok(new_cfg)
}

#[tauri::command]
pub fn get_agent_sound_enabled(app: AppHandle) -> bool {
    config::load(&app).agent_sound_enabled
}

#[tauri::command]
pub fn set_agent_sound_enabled(app: AppHandle, enabled: bool) {
    let mut cfg = config::load(&app);
    cfg.agent_sound_enabled = enabled;
    let _ = config::save(&app, &cfg);
}

#[derive(Serialize)]
pub struct HookConfig {
    pub script_path: String,
    pub json: String,
}

#[tauri::command]
pub fn get_agent_hook_config(app: AppHandle) -> Result<HookConfig, String> {
    let path = hook_script_path(&app)?.to_string_lossy().to_string();
    Ok(HookConfig {
        json: hook_config_json(&path),
        script_path: path,
    })
}

#[cfg(test)]
mod tests;
