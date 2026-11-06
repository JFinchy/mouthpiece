//! The running app's end of the `mouthpiece` command and its MCP server
//! (see `cli.rs`): a Unix socket in the app's data folder, which only this
//! user can open.
//!
//! One JSON request per connection, one JSON reply:
//! - `{"cmd":"status"}`
//! - `{"cmd":"listen","prompt":"…","timeout_s":120}`: records a dictation
//!   (the user stops it with their shortcut, as usual) and returns the
//!   text instead of pasting it. The widget shows the prompt.
//! - `{"cmd":"vocab_add","terms":["…"]}` and `{"cmd":"vocab_list"}`

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const SOCKET_NAME: &str = "mouthpiece.sock";
const DEFAULT_LISTEN_TIMEOUT_S: u64 = 180;
const MAX_LISTEN_TIMEOUT_S: u64 = 900;
const MAX_PROMPT_CHARS: usize = 200;
const MAX_TERMS: usize = 50;
const MAX_TERM_CHARS: usize = 60;

/// A dictation whose text goes back to a `listen` caller instead of being
/// pasted. Dropping the sender (deleted, nothing heard, failed) tells the
/// caller there's no answer.
pub struct Capture {
    pub tx: mpsc::Sender<String>,
}

#[derive(Default)]
pub struct CliState {
    pending: Mutex<Option<Capture>>,
}

impl CliState {
    /// Taken when the recording stops, like a pending agent reply.
    pub fn take_pending(&self) -> Option<Capture> {
        self.pending.lock().unwrap().take()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }
}

#[derive(Deserialize)]
struct Request {
    cmd: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    timeout_s: Option<u64>,
    #[serde(default)]
    terms: Vec<String>,
}

#[derive(Serialize)]
struct Status {
    version: &'static str,
    state: &'static str,
    waiting_agents: Vec<String>,
}

/// Where the `mouthpiece` command is kept (Settings shows it).
pub fn command_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("bin").join("mouthpiece"))
}

pub fn socket_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(SOCKET_NAME))
}

/// Starts listening; called once at launch. A stale socket from a crashed
/// run is replaced, but not one another running copy is answering on.
pub fn start(app: &AppHandle) {
    let Some(path) = socket_path(app) else {
        return;
    };
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            crate::applog!(
                "cli: another Mouthpiece is answering on the socket, not taking it over"
            );
            return;
        }
        let _ = std::fs::remove_file(&path);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            crate::applog!("cli: couldn't open the socket: {err}");
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    if let Some(dir) = path.parent() {
        if let Err(err) = crate::cli::install_wrapper(dir) {
            crate::applog!("cli: couldn't install the mouthpiece command: {err}");
        }
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            std::thread::spawn(move || handle(&app, stream));
        }
    });
}

fn handle(app: &AppHandle, stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.len() > 64 * 1024 {
        return;
    }
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(req) => respond(app, req),
        Err(err) => json!({"ok": false, "error": format!("bad request: {err}")}),
    };
    let mut out = &stream;
    let _ = writeln!(out, "{reply}");
}

fn respond(app: &AppHandle, req: Request) -> Value {
    match req.cmd.as_str() {
        "status" => json!({"ok": true, "status": status(app)}),
        "listen" => match listen(app, req.prompt, req.timeout_s) {
            Ok(text) => json!({"ok": true, "text": text}),
            Err(err) => json!({"ok": false, "error": err}),
        },
        "vocab_add" => match vocab_add(app, &req.terms) {
            Ok(added) => json!({"ok": true, "added": added}),
            Err(err) => json!({"ok": false, "error": err}),
        },
        "vocab_list" => json!({"ok": true, "terms": crate::config::load(app).vocabulary}),
        other => json!({"ok": false, "error": format!("unknown command {other:?}")}),
    }
}

fn status(app: &AppHandle) -> Status {
    use crate::capture::CaptureState;
    let state = match app
        .state::<crate::recording::RecordingState>()
        .capture
        .state()
    {
        CaptureState::Idle => "idle",
        CaptureState::Dictating { .. } => "dictating",
        CaptureState::Meeting { .. } => "meeting",
        CaptureState::Enrolling { .. } => "enrolling",
    };
    let waiting_agents = crate::agents::get_waiting_agents(app.state())
        .into_iter()
        .map(|a| a.folder)
        .collect();
    Status {
        version: env!("CARGO_PKG_VERSION"),
        state,
        waiting_agents,
    }
}

fn clean_prompt(prompt: Option<String>) -> Option<String> {
    let flat: String = prompt?
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    Some(if flat.chars().count() > MAX_PROMPT_CHARS {
        format!(
            "{}…",
            flat.chars().take(MAX_PROMPT_CHARS - 1).collect::<String>()
        )
    } else {
        flat
    })
}

/// Records one dictation and returns its text. Refuses while anything else
/// is recording, so it never hijacks a dictation the user started.
fn listen(
    app: &AppHandle,
    prompt: Option<String>,
    timeout_s: Option<u64>,
) -> Result<String, String> {
    use crate::capture::{CaptureState, Request, Trigger};
    let recording = app.state::<crate::recording::RecordingState>();
    let cli = app.state::<CliState>();
    if !matches!(recording.capture.state(), CaptureState::Idle) || cli.has_pending() {
        return Err("Mouthpiece is busy recording. Try again when it's idle.".to_string());
    }
    let prompt = clean_prompt(prompt);
    let (tx, rx) = mpsc::channel();
    *cli.pending.lock().unwrap() = Some(Capture { tx });
    crate::applog!(
        "cli: listening for an answer (prompt {} chars)",
        prompt.as_deref().map_or(0, |p| p.chars().count())
    );
    let _ = app.emit("agent-question", prompt.clone().unwrap_or_default());
    crate::recording::request_dictation(app, Request::StartDictation(Trigger::DeepLink));
    if !matches!(recording.capture.state(), CaptureState::Dictating { .. }) {
        cli.take_pending();
        let _ = app.emit("agent-question", Value::Null);
        return Err("Couldn't start recording.".to_string());
    }

    let timeout = Duration::from_secs(
        timeout_s
            .unwrap_or(DEFAULT_LISTEN_TIMEOUT_S)
            .clamp(5, MAX_LISTEN_TIMEOUT_S),
    );
    let result = match rx.recv_timeout(timeout) {
        Ok(text) => Ok(text),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("No answer: the recording was deleted or nothing was heard.".to_string())
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Still recording for us: throw it away rather than paste it.
            if cli.take_pending().is_some() {
                crate::recording::request_dictation(app, Request::DiscardDictation);
            }
            Err(format!("No answer within {} s.", timeout.as_secs()))
        }
    };
    let _ = app.emit("agent-question", Value::Null);
    result
}

/// Adds terms to the vocabulary, skipping ones already there. Returns the
/// ones added.
fn vocab_add(app: &AppHandle, terms: &[String]) -> Result<Vec<String>, String> {
    if terms.len() > MAX_TERMS {
        return Err(format!("At most {MAX_TERMS} terms at a time."));
    }
    let mut cfg = crate::config::load(app);
    let mut added = Vec::new();
    for term in terms {
        let term = term.trim();
        if term.is_empty()
            || term.chars().count() > MAX_TERM_CHARS
            || term.chars().any(char::is_control)
        {
            continue;
        }
        let exists = cfg
            .vocabulary
            .iter()
            .chain(added.iter())
            .any(|t: &String| t.eq_ignore_ascii_case(term));
        if !exists {
            added.push(term.to_string());
        }
    }
    if !added.is_empty() {
        cfg.vocabulary.extend(added.iter().cloned());
        crate::config::save(app, &cfg)?;
        crate::applog!("cli: added {} vocabulary terms", added.len());
        let _ = app.emit("vocabulary-changed", cfg.vocabulary.clone());
    }
    Ok(added)
}

#[derive(Serialize)]
pub struct CommandInfo {
    pub command_path: Option<String>,
    pub running: bool,
}

/// Where the `mouthpiece` command is, for Settings > Coding agents.
#[tauri::command]
pub fn get_cli_info(app: AppHandle) -> CommandInfo {
    CommandInfo {
        command_path: command_path(&app).map(|p| p.to_string_lossy().to_string()),
        running: socket_path(&app).is_some_and(|p| UnixStream::connect(p).is_ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_flattened_and_capped() {
        assert_eq!(
            clean_prompt(Some("  Which\nbranch?\t".into())).as_deref(),
            Some("Which branch?")
        );
        assert_eq!(clean_prompt(Some("   ".into())), None);
        let long = clean_prompt(Some("x".repeat(500))).unwrap();
        assert_eq!(long.chars().count(), MAX_PROMPT_CHARS);
    }
}
