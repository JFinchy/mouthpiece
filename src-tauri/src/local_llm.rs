//! LLMs that are already on this Mac but not pulled through Ollama:
//!
//! - **Local servers** with an OpenAI-compatible API — LM Studio, llama.cpp's
//!   `llama-server`, Jan. Their models are listed next to Ollama's and used
//!   the same way. Stored model names look like `lmstudio::google/gemma-3-4b`
//!   (Ollama names never contain `::`).
//! - **`.gguf` files** (from Hugging Face, a shared drive, LM Studio's
//!   folder...), which can be added to the local Ollama in one click.
//!
//! Both exist for Macs where Ollama's own downloads are blocked (corporate
//! proxy, an allow-listed model source) but a model is already on disk.
//!
//! Nothing leaves the Mac: servers are only reached on 127.0.0.1, redirects
//! are never followed (so a local server can't bounce dictated text to
//! another host), and an imported file is copied into the local Ollama only.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Separates a server's key from its own model id in a stored name.
const SEP: &str = "::";

pub struct Server {
    pub key: &'static str,
    pub name: &'static str,
    pub base: &'static str,
}

/// Default ports. Only loopback: anything else would send dictated text
/// off the Mac.
pub static SERVERS: [Server; 3] = [
    Server {
        key: "lmstudio",
        name: "LM Studio",
        base: "http://127.0.0.1:1234",
    },
    Server {
        key: "llamacpp",
        name: "llama.cpp server",
        base: "http://127.0.0.1:8080",
    },
    Server {
        key: "jan",
        name: "Jan",
        base: "http://127.0.0.1:1337",
    },
];

pub fn model_id(server: &Server, model: &str) -> String {
    format!("{}{SEP}{model}", server.key)
}

/// The server a stored model name belongs to, and that server's model id.
/// `None` for Ollama names.
pub fn parse(id: &str) -> Option<(&'static Server, &str)> {
    let (key, model) = id.split_once(SEP)?;
    let server = SERVERS.iter().find(|s| s.key == key)?;
    (!model.is_empty()).then_some((server, model))
}

/// What Settings shows for a stored model name ("gemma-3-4b · LM Studio").
pub fn label(id: &str) -> Option<String> {
    parse(id).map(|(server, model)| format!("{model} · {}", server.name))
}

/// Never follows redirects: a local server answering with a redirect to a
/// remote host must not take the prompt there.
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(2))
        .timeout(timeout)
        .build()
}

/// With redirects off, ureq hands a 3xx back as a success; it's an error
/// here (and is never followed).
fn no_redirect(name: &str, resp: ureq::Response) -> Result<ureq::Response, String> {
    if (300..400).contains(&resp.status()) {
        Err(format!(
            "{name} answered HTTP {} (a redirect, which Mouthpiece doesn't follow)",
            resp.status()
        ))
    } else {
        Ok(resp)
    }
}

// ------------------------------------------------------------ discovery

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// Embedding models answer `/v1/models` too but can't write text.
fn usable(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    !lower.trim().is_empty() && !lower.contains("embed")
}

fn models_at(base: &str) -> Option<Vec<String>> {
    let resp = agent(Duration::from_secs(2))
        .get(&format!("{base}/v1/models"))
        .call()
        .ok()
        .and_then(|resp| no_redirect("", resp).ok())?;
    let list: ModelList = resp.into_json().ok()?;
    Some(
        list.data
            .into_iter()
            .map(|m| m.id)
            .filter(|id| usable(id))
            .collect(),
    )
}

pub struct Found {
    pub server: &'static Server,
    pub models: Vec<String>,
}

/// Every local server that's running, with its models. Probed in parallel
/// so a missing server costs one refused connection, not a timeout each.
pub fn reachable() -> Vec<Found> {
    let probes: Vec<_> = SERVERS
        .iter()
        .map(|server| (server, std::thread::spawn(move || models_at(server.base))))
        .collect();
    probes
        .into_iter()
        .filter_map(|(server, probe)| {
            Some(Found {
                server,
                models: probe.join().ok()??,
            })
        })
        .collect()
}

// ------------------------------------------------------------ generation

fn chat_body(
    model: &str,
    prompt: &str,
    stream: bool,
    max_tokens: Option<u32>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "stream": stream,
        // Cleanup should be faithful, not creative.
        "temperature": 0.2,
    });
    if let Some(max) = max_tokens {
        body["max_tokens"] = max.into();
    }
    body
}

fn request_error(server: &str, err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body: String = resp
                .into_string()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            format!("{server} answered HTTP {code}: {}", body.trim())
        }
        ureq::Error::Transport(t) => {
            format!(
                "{server} isn't reachable (is its local server running?): {}",
                t.kind()
            )
        }
    }
}

/// Reasoning models served this way may put their thinking inline.
pub fn strip_think(text: &str) -> String {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return rest[end + "</think>".len()..].trim().to_string();
        }
    }
    text.trim().to_string()
}

/// One whole answer, for the journal and boilerplate (not streamed).
pub fn complete(
    server: &Server,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, String> {
    complete_at(server.base, server.name, model, prompt, timeout)
}

fn complete_at(
    base: &str,
    name: &str,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, String> {
    let resp = agent(timeout)
        .post(&format!("{base}/v1/chat/completions"))
        .send_json(chat_body(model, prompt, false, None))
        .map_err(|e| request_error(name, e))
        .and_then(|resp| no_redirect(name, resp))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("couldn't read {name}'s answer: {e}"))?;
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    Ok(strip_think(text))
}

/// Loads the model ahead of time (LM Studio and Jan load on first use)
/// with a one-token request. Failures are ignored, like Ollama's prewarm.
pub fn prewarm(server: &'static Server, model: String) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let result = agent(Duration::from_secs(60))
            .post(&format!("{}/v1/chat/completions", server.base))
            .send_json(chat_body(&model, "Hi", false, Some(1)));
        match result {
            Ok(_) => crate::applog!(
                "llm: prewarmed {model} on {} in {}ms",
                server.name,
                started.elapsed().as_millis()
            ),
            Err(err) => crate::applog!(
                "llm: prewarm of {model} on {} skipped: {}",
                server.name,
                request_error(server.name, err)
            ),
        }
    });
}

pub enum Piece {
    Text(String),
    Done,
    Failed(String),
}

/// Streams an answer as server-sent events, handing each piece to `emit`
/// until it returns `false` (the caller hung up) or the answer ends.
pub fn stream(server: &Server, model: &str, prompt: &str, emit: &mut dyn FnMut(Piece) -> bool) {
    stream_at(server.base, server.name, model, prompt, emit)
}

fn stream_at(
    base: &str,
    name: &str,
    model: &str,
    prompt: &str,
    emit: &mut dyn FnMut(Piece) -> bool,
) {
    let resp = match agent(Duration::from_secs(30))
        .post(&format!("{base}/v1/chat/completions"))
        .send_json(chat_body(model, prompt, true, None))
        .map_err(|err| request_error(name, err))
        .and_then(|resp| no_redirect(name, resp))
    {
        Ok(resp) => resp,
        Err(err) => {
            emit(Piece::Failed(err));
            return;
        }
    };
    for line in std::io::BufReader::new(resp.into_reader()).lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                emit(Piece::Failed(format!("{name}'s stream broke: {e}")));
                return;
            }
        };
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            emit(Piece::Done);
            return;
        }
        let value: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                emit(Piece::Failed(format!("couldn't read {name}'s answer: {e}")));
                return;
            }
        };
        if let Some(err) = value.get("error") {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            emit(Piece::Failed(format!("{name}: {message}")));
            return;
        }
        // Only `content`: a separate `reasoning_content` is the model's
        // thinking, not its answer.
        if let Some(text) = value
            .pointer("/choices/0/delta/content")
            .and_then(|c| c.as_str())
        {
            if !text.is_empty() && !emit(Piece::Text(text.to_string())) {
                return;
            }
        }
    }
    emit(Piece::Failed(format!("{name} closed the stream early")));
}

// ------------------------------------------------------- model files

/// Smaller `.gguf` files are tokenizers, vision projectors or test files.
const MIN_MODEL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_DEPTH: usize = 6;
const MAX_FILES: usize = 50;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ModelFile {
    pub path: String,
    pub file_name: String,
    pub size_gb: f32,
    /// Where it was found, for the list ("LM Studio", "Hugging Face cache").
    pub location: String,
    /// The name it gets (or already has) in Ollama.
    pub ollama_name: String,
    pub in_ollama: bool,
}

/// Folders models usually end up in. None are behind macOS's Files and
/// Folders privacy prompt (Downloads, Documents and Desktop are; files
/// there are added with the file picker instead). The server key skips a
/// folder whose server is running, since its models are listed already.
fn search_roots(home: &Path) -> Vec<(PathBuf, &'static str, Option<&'static str>)> {
    vec![
        (home.join(".lmstudio/models"), "LM Studio", Some("lmstudio")),
        (
            home.join(".cache/lm-studio/models"),
            "LM Studio",
            Some("lmstudio"),
        ),
        (
            home.join("Library/Application Support/Jan/data/models"),
            "Jan",
            Some("jan"),
        ),
        (home.join("jan/models"), "Jan", Some("jan")),
        (
            home.join(".cache/huggingface/hub"),
            "Hugging Face cache",
            None,
        ),
        (
            home.join(".cache/llama.cpp"),
            "llama.cpp cache",
            Some("llamacpp"),
        ),
        (home.join("models"), "~/models", None),
        (home.join(".ollama-import"), "~/.ollama-import", None),
    ]
}

/// A name Ollama accepts, from a file name:
/// "gemma-3-4b-it-Q4_K_M.gguf" -> "gemma-3-4b-it-q4_k_m".
pub fn ollama_name_for(file_name: &str) -> String {
    let stem = file_name
        .len()
        .checked_sub(".gguf".len())
        .filter(|&cut| file_name[cut..].eq_ignore_ascii_case(".gguf"))
        .map_or(file_name, |cut| &file_name[..cut]);
    let mut name: String = stem
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    name = name
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(80)
        .collect();
    if name.is_empty() {
        "imported-model".to_string()
    } else {
        name
    }
}

fn is_model_file(path: &Path, size: u64) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".gguf")
        && size >= MIN_MODEL_BYTES
        // Vision projectors go with a model; they aren't one.
        && !lower.contains("mmproj")
        // Split models ("-00001-of-00003.gguf") can't be imported as one file.
        && !lower.contains("-of-0")
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<(PathBuf, u64)>) {
    if depth > MAX_DEPTH || found.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Directories aren't followed through symlinks (no loops); files
        // are, since the Hugging Face cache links snapshots to blobs.
        let Ok(link_meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if link_meta.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.') {
                walk(&path, depth + 1, found);
            }
        } else if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() && is_model_file(&path, meta.len()) {
                found.push((path, meta.len()));
            }
        }
        if found.len() >= MAX_FILES {
            return;
        }
    }
}

fn describe(path: &Path, size: u64, location: &str, ollama: &[String]) -> ModelFile {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ollama_name = ollama_name_for(&file_name);
    let in_ollama = ollama
        .iter()
        .any(|m| m == &ollama_name || *m == format!("{ollama_name}:latest"));
    ModelFile {
        path: path.to_string_lossy().into_owned(),
        file_name,
        size_gb: size as f32 / 1e9,
        location: location.to_string(),
        ollama_name,
        in_ollama,
    }
}

/// Model files in the usual folders, skipping folders whose server is
/// running (their models are listed already).
pub fn find_model_files(home: &Path, running: &[&str], ollama: &[String]) -> Vec<ModelFile> {
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for (root, location, server) in search_roots(home) {
        if server.is_some_and(|key| running.contains(&key)) {
            continue;
        }
        let mut found = Vec::new();
        walk(&root, 0, &mut found);
        for (path, size) in found {
            // The same blob can show up twice (a symlinked snapshot, or a
            // case-insensitive duplicate path).
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen.insert(key) {
                files.push(describe(&path, size, location, ollama));
            }
        }
    }
    files.truncate(MAX_FILES);
    files
}

#[tauri::command(async)]
pub fn find_local_model_files() -> Vec<ModelFile> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return vec![];
    };
    let running: Vec<&str> = reachable().iter().map(|f| f.server.key).collect();
    let ollama = crate::llm::ollama_models().unwrap_or_default();
    find_model_files(&home, &running, &ollama)
}

/// Checks the file is a GGUF model: by name, and by its magic bytes.
fn check_gguf(path: &Path) -> Result<u64, String> {
    let is_gguf_name = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
    let meta = std::fs::metadata(path).map_err(|e| format!("Can't read that file: {e}"))?;
    if !is_gguf_name || !meta.is_file() {
        return Err("That isn't a .gguf model file.".into());
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| format!("Can't read that file: {e}"))?;
    if &magic != b"GGUF" {
        return Err("That file doesn't look like a GGUF model.".into());
    }
    Ok(meta.len())
}

/// Shows the macOS file picker for a `.gguf` file anywhere (Downloads
/// included: picking a file grants access to it).
#[tauri::command(async)]
pub fn choose_model_file(app: AppHandle) -> Result<Option<ModelFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose a model file (.gguf)")
        .add_filter("GGUF model", &["gguf"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let size = check_gguf(&path)?;
    let ollama = crate::llm::ollama_models().unwrap_or_default();
    Ok(Some(describe(&path, size, "Chosen file", &ollama)))
}

/// Reports bytes read as a percentage, for hashing and uploading.
struct Progress<R, F> {
    inner: R,
    done: u64,
    total: u64,
    last: Option<u32>,
    on_percent: F,
}

impl<R: Read, F: FnMut(u32)> Read for Progress<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        let percent = (self.done * 100 / self.total.max(1)).min(100) as u32;
        if self.last != Some(percent) {
            self.last = Some(percent);
            (self.on_percent)(percent);
        }
        Ok(n)
    }
}

fn sha256_hex(path: &Path, total: u64, on_percent: impl FnMut(u32)) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut reader = Progress {
        inner: std::io::BufReader::with_capacity(1 << 20, file),
        done: 0,
        total,
        last: None,
        on_percent,
    };
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Copies a `.gguf` file into the local Ollama as model `name`, the API
/// equivalent of `ollama create name -f Modelfile` with `FROM <file>`:
/// upload the file as a blob (skipped if Ollama has it), then create a
/// model from it. Ollama reads the chat template from the file itself.
pub fn import_into_ollama(
    base: &str,
    path: &Path,
    name: &str,
    progress: &dyn Fn(&str, Option<u32>),
) -> Result<(), String> {
    let size = check_gguf(path)?;
    let unreachable = |e: ureq::Error| match e {
        ureq::Error::Transport(_) => {
            "Ollama isn't running. Start Ollama and try again (or open the file in LM Studio and \
             turn on its local server)."
                .to_string()
        }
        other => request_error("Ollama", other),
    };

    progress("Checking the file", Some(0));
    let digest = sha256_hex(path, size, |p| progress("Checking the file", Some(p)))?;
    let blob_url = format!("{base}/api/blobs/sha256:{digest}");

    let have_blob = match agent(Duration::from_secs(10)).head(&blob_url).call() {
        Ok(resp) => no_redirect("Ollama", resp).map(|_| true)?,
        Err(ureq::Error::Status(404, _)) => false,
        Err(e) => return Err(unreachable(e)),
    };
    if !have_blob {
        progress("Copying into Ollama", Some(0));
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let body = Progress {
            inner: std::io::BufReader::with_capacity(1 << 20, file),
            done: 0,
            total: size,
            last: None,
            on_percent: |p| progress("Copying into Ollama", Some(p)),
        };
        // Per-read/write timeouts, not a total: a 5GB copy takes a while.
        ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(2))
            .timeout_read(Duration::from_secs(120))
            .timeout_write(Duration::from_secs(120))
            .build()
            .post(&blob_url)
            .set("Content-Length", &size.to_string())
            .send(body)
            .map_err(unreachable)
            .and_then(|resp| no_redirect("Ollama", resp))?;
    }

    progress("Setting up the model", None);
    let resp = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(600))
        .build()
        .post(&format!("{base}/api/create"))
        .send_json(serde_json::json!({
            "model": name,
            "files": { format!("{name}.gguf"): format!("sha256:{digest}") },
            "stream": false,
        }))
        .map_err(|e| match e {
            // Ollama before 0.5.5 had no `files` field.
            ureq::Error::Status(400, resp) => format!(
                "Ollama refused the model ({}). Updating Ollama usually fixes this.",
                resp.into_string().unwrap_or_default().trim()
            ),
            other => unreachable(other),
        })
        .and_then(|resp| no_redirect("Ollama", resp))?;
    let answer: serde_json::Value = resp.into_json().unwrap_or_default();
    if let Some(err) = answer.get("error").and_then(|e| e.as_str()) {
        return Err(format!("Ollama couldn't create the model: {err}"));
    }
    Ok(())
}

/// Adds a model file to Ollama in the background; Settings follows along
/// through `llm-import-progress` / `-complete` / `-error`.
#[tauri::command]
pub fn import_model_file(app: AppHandle, path: String) {
    std::thread::spawn(move || {
        let file = PathBuf::from(&path);
        let name = ollama_name_for(
            &file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        crate::applog!("llm: importing a model file into Ollama as {name}");
        let report = |stage: &str, percent: Option<u32>| {
            let _ = app.emit(
                "llm-import-progress",
                serde_json::json!({ "path": path, "stage": stage, "percent": percent }),
            );
        };
        match import_into_ollama(crate::llm::OLLAMA_BASE, &file, &name, &report) {
            Ok(()) => {
                crate::applog!("llm: imported {name} into Ollama");
                let _ = app.emit(
                    "llm-import-complete",
                    serde_json::json!({ "path": path, "model": name }),
                );
            }
            Err(error) => {
                crate::applog!("llm: import of {name} failed: {error}");
                let _ = app.emit(
                    "llm-import-error",
                    serde_json::json!({ "path": path, "error": error }),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn stored_names_round_trip_and_ollama_names_are_left_alone() {
        let id = model_id(&SERVERS[0], "google/gemma-3-4b");
        assert_eq!(id, "lmstudio::google/gemma-3-4b");
        let (server, model) = parse(&id).unwrap();
        assert_eq!((server.key, model), ("lmstudio", "google/gemma-3-4b"));
        assert!(parse("gemma3:4b").is_none());
        assert!(parse("hf.co/unsloth/gemma-3-4b-it-GGUF:Q4_K_M").is_none());
        assert!(parse("unknown::model").is_none());
        assert!(parse("lmstudio::").is_none());
        assert_eq!(label(&id).unwrap(), "google/gemma-3-4b · LM Studio");
    }

    #[test]
    fn every_server_is_on_loopback() {
        for server in &SERVERS {
            assert!(
                server.base.starts_with("http://127.0.0.1:"),
                "{}",
                server.base
            );
        }
    }

    #[test]
    fn ollama_names_are_valid() {
        assert_eq!(
            ollama_name_for("gemma-3-4b-it-Q4_K_M.gguf"),
            "gemma-3-4b-it-q4_k_m"
        );
        assert_eq!(ollama_name_for("My Model (v2).GGUF"), "my-model--v2");
        assert_eq!(ollama_name_for("---.gguf"), "imported-model");
    }

    #[test]
    fn strips_inline_thinking() {
        assert_eq!(strip_think("<think>hmm</think>\nHello."), "Hello.");
        assert_eq!(strip_think("  Hello. "), "Hello.");
        assert_eq!(strip_think("<think>never closed"), "<think>never closed");
    }

    /// A one-request fake server: answers with `status` and `body`, and
    /// hands back the request it got.
    fn fake_server(status: &'static str, body: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if request.len() >= head_end + 4 + length || n == 0 {
                        break;
                    }
                } else if n == 0 {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        (base, rx)
    }

    fn collect(base: &str) -> (String, Option<String>) {
        let mut text = String::new();
        let mut failed = None;
        stream_at(base, "Test", "m", "clean this", &mut |piece| {
            match piece {
                Piece::Text(t) => text.push_str(&t),
                Piece::Done => {}
                Piece::Failed(e) => failed = Some(e),
            }
            true
        });
        (text, failed)
    }

    #[test]
    fn streams_openai_style_chunks_and_ignores_reasoning() {
        let body = [
            r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#,
            r#"data: {"choices":[{"delta":{"reasoning_content":"thinking"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Hello, "}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"world."}}]}"#,
            "data: [DONE]",
            "",
        ]
        .join("\n\n");
        let (base, request) = fake_server("200 OK", body);
        let (text, failed) = collect(&base);
        assert_eq!(failed, None);
        assert_eq!(text, "Hello, world.");
        let request = request.recv().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions"));
        assert!(request.contains(r#""stream":true"#));
        assert!(request.contains("clean this"));
    }

    #[test]
    fn a_stream_without_done_is_a_failure() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n".to_string();
        let (base, _) = fake_server("200 OK", body);
        let (_, failed) = collect(&base);
        assert!(failed.unwrap().contains("closed the stream early"));
    }

    #[test]
    fn redirects_are_not_followed() {
        let (base, _) = fake_server(
            "307 Temporary Redirect\r\nLocation: http://example.com/",
            String::new(),
        );
        let (_, failed) = collect(&base);
        assert!(failed.unwrap().contains("HTTP 307 (a redirect"));
    }

    #[test]
    fn lists_models_without_embeddings() {
        let body = r#"{"data":[{"id":"google/gemma-3-4b"},{"id":"text-embedding-nomic-embed-text-v1.5"}]}"#;
        let (base, _) = fake_server("200 OK", body.to_string());
        assert_eq!(
            models_at(&base).unwrap(),
            vec!["google/gemma-3-4b".to_string()]
        );
    }

    #[test]
    fn completes_without_streaming() {
        let body =
            r#"{"choices":[{"message":{"role":"assistant","content":"Fix the login bug"}}]}"#;
        let (base, request) = fake_server("200 OK", body.to_string());
        let text = complete_at(&base, "Test", "m", "summarize", Duration::from_secs(5)).unwrap();
        assert_eq!(text, "Fix the login bug");
        assert!(request.recv().unwrap().contains(r#""stream":false"#));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mp-local-llm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A sparse file: the right size without writing gigabytes.
    fn fake_gguf(path: &Path, size: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(b"GGUF").unwrap();
        f.set_len(size).unwrap();
    }

    #[test]
    fn finds_model_files_and_skips_the_rest() {
        let home = temp_dir("find");
        let big = MIN_MODEL_BYTES + 1;
        fake_gguf(
            &home.join(".lmstudio/models/lmstudio-community/gemma/gemma-3-4b-Q4_K_M.gguf"),
            big,
        );
        fake_gguf(
            &home.join(".lmstudio/models/lmstudio-community/gemma/mmproj-gemma.gguf"),
            big,
        );
        fake_gguf(&home.join("models/tiny.gguf"), 1024);
        fake_gguf(&home.join("models/split-00001-of-00002.gguf"), big);
        fake_gguf(&home.join("models/Qwen.gguf"), big);

        let files = find_model_files(&home, &[], &["qwen:latest".to_string()]);
        let names: Vec<(&str, &str, bool)> = files
            .iter()
            .map(|f| (f.file_name.as_str(), f.location.as_str(), f.in_ollama))
            .collect();
        assert_eq!(
            names,
            vec![
                ("gemma-3-4b-Q4_K_M.gguf", "LM Studio", false),
                ("Qwen.gguf", "~/models", true),
            ]
        );

        // LM Studio's own folder is skipped while its server lists them.
        let files = find_model_files(&home, &["lmstudio"], &[]);
        assert_eq!(files.len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_files_that_are_not_gguf_models() {
        let dir = temp_dir("check");
        let fake = dir.join("notes.gguf");
        std::fs::write(&fake, b"hello").unwrap();
        assert!(check_gguf(&fake).unwrap_err().contains("doesn't look like"));
        let wrong_ext = dir.join("model.bin");
        fake_gguf(&wrong_ext, 16);
        assert!(check_gguf(&wrong_ext)
            .unwrap_err()
            .contains("isn't a .gguf"));
        let real = dir.join("model.gguf");
        fake_gguf(&real, 16);
        assert_eq!(check_gguf(&real).unwrap(), 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fake Ollama for the import: answers each request in turn, recording
    /// the method and path of each.
    fn fake_ollama(answers: Vec<(&'static str, &'static str)>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = Vec::new();
                let mut buf = vec![0u8; 1 << 16];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    let Some(head_end) = text.find("\r\n\r\n") else {
                        if n == 0 {
                            break;
                        }
                        continue;
                    };
                    let length = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if request.len() >= head_end + 4 + length || n == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let first = text.lines().next().unwrap_or_default().to_string();
                let body_start = text.find("\r\n\r\n").map_or(text.len(), |i| i + 4);
                let summary = if first.contains("/api/create") {
                    format!("{first} {}", &text[body_start..])
                } else {
                    format!("{first} bytes={}", request.len() - body_start)
                };
                let _ = tx.send(summary);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (base, rx)
    }

    #[test]
    fn imports_a_file_into_ollama() {
        let dir = temp_dir("import");
        let path = dir.join("Gemma-3-4B.gguf");
        fake_gguf(&path, 4096);
        let (base, requests) = fake_ollama(vec![
            ("404 Not Found", ""),
            ("201 Created", ""),
            ("200 OK", r#"{"status":"success"}"#),
        ]);
        let stages = std::sync::Mutex::new(Vec::new());
        import_into_ollama(&base, &path, "gemma-3-4b", &|stage, _| {
            let mut stages = stages.lock().unwrap();
            if stages.last().map(String::as_str) != Some(stage) {
                stages.push(stage.to_string());
            }
        })
        .unwrap();

        let head = requests.recv().unwrap();
        assert!(head.starts_with("HEAD /api/blobs/sha256:"), "{head}");
        let upload = requests.recv().unwrap();
        assert!(upload.starts_with("POST /api/blobs/sha256:"), "{upload}");
        assert!(upload.ends_with("bytes=4096"), "{upload}");
        let create = requests.recv().unwrap();
        assert!(create.starts_with("POST /api/create"), "{create}");
        assert!(create.contains(r#""model":"gemma-3-4b""#), "{create}");
        assert!(create.contains(r#""gemma-3-4b.gguf":"sha256:"#), "{create}");
        assert_eq!(
            *stages.lock().unwrap(),
            vec![
                "Checking the file",
                "Copying into Ollama",
                "Setting up the model"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_skips_the_upload_when_ollama_has_the_file() {
        let dir = temp_dir("import-have");
        let path = dir.join("m.gguf");
        fake_gguf(&path, 64);
        let (base, requests) =
            fake_ollama(vec![("200 OK", ""), ("200 OK", r#"{"status":"success"}"#)]);
        import_into_ollama(&base, &path, "m", &|_, _| {}).unwrap();
        assert!(requests.recv().unwrap().starts_with("HEAD "));
        assert!(requests.recv().unwrap().starts_with("POST /api/create"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_explains_when_ollama_is_not_running() {
        let dir = temp_dir("import-down");
        let path = dir.join("m.gguf");
        fake_gguf(&path, 64);
        // Bound then dropped: nothing listens on this port.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = import_into_ollama(&format!("http://127.0.0.1:{port}"), &path, "m", &|_, _| {})
            .unwrap_err();
        assert!(err.contains("Ollama isn't running"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
