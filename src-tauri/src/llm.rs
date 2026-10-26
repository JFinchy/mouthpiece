use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

const OLLAMA_GENERATE_URL: &str = "http://localhost:11434/api/generate";
const OLLAMA_TAGS_URL: &str = "http://localhost:11434/api/tags";
const OLLAMA_PULL_URL: &str = "http://localhost:11434/api/pull";

/// How long Ollama keeps a model loaded after a request. Its default is 5
/// minutes, so the first dictation after a short break paid a multi-second
/// cold load; dictation comes in bursts spread across a working session.
const KEEP_ALIVE: &str = "30m";

/// Minimum gap between prewarm requests for the same model. Well inside
/// `KEEP_ALIVE`, so a model in steady use is never allowed to unload.
const PREWARM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

static LAST_PREWARM: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// Starts loading `model` into Ollama's memory in the background, so the
/// load overlaps with the user still talking instead of being added to the
/// wait after they stop. An empty generate request loads without producing
/// output. Throttled per model; failures (Ollama not running) are ignored,
/// since refinement reports its own error if it's actually needed.
pub fn prewarm(model: &str) {
    if model.is_empty() {
        return;
    }
    let now = Instant::now();
    {
        let mut last = LAST_PREWARM
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if last
            .get(model)
            .is_some_and(|t| now.duration_since(*t) < PREWARM_INTERVAL)
        {
            return;
        }
        last.insert(model.to_string(), now);
    }
    if require_local(model).is_err() {
        return;
    }
    let model = model.to_string();
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = ureq::post(OLLAMA_GENERATE_URL)
            .timeout(std::time::Duration::from_secs(60))
            .send_json(serde_json::json!({ "model": model, "keep_alive": KEEP_ALIVE }));
        match result {
            Ok(_) => crate::applog!(
                "llm: prewarmed {model} in {}ms",
                started.elapsed().as_millis()
            ),
            Err(err) => crate::applog!("llm: prewarm of {model} skipped: {err}"),
        }
    });
}

/// Every model some mode would refine with, deduplicated.
pub fn models_in_use(cfg: &crate::config::AppConfig) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for mode in &cfg.modes {
        let model = match &mode.llm_refinement {
            crate::modes::LlmRefinement::Off => continue,
            crate::modes::LlmRefinement::Global => cfg.llm_model.clone(),
            crate::modes::LlmRefinement::Model(id) => id.clone(),
        };
        if !models.contains(&model) {
            models.push(model);
        }
    }
    models
}

/// Last observed `refine()` round-trip time per model, in milliseconds —
/// surfaced in Settings so a user picking between e.g. gemma3:1b and
/// mistral:7b can see the real speed difference on their own hardware
/// rather than guessing from parameter count. Process-lifetime only
/// (resets on restart); persisting it wasn't worth the complexity since
/// this is a menu bar app that's typically left running.
static LAST_LATENCY_MS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

fn latency_store() -> &'static Mutex<HashMap<String, u64>> {
    LAST_LATENCY_MS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_latency(model: &str, ms: u64) {
    latency_store()
        .lock()
        .unwrap()
        .insert(model.to_string(), ms);
}

fn last_latency_ms(model: &str) -> Option<u64> {
    latency_store().lock().unwrap().get(model).copied()
}

pub fn default_model() -> String {
    "qwen3.5:4b".to_string()
}

struct CatalogEntry {
    id: &'static str,
    label: &'static str,
    size_gb: f32,
}

/// Recommended models for dictation refinement — deliberately small/fast
/// (1-4B range), since this runs after every recording and needs to feel
/// instant, not deliberate. Ollama's official library names, current as of
/// writing; tags can move, so pulling gracefully surfaces a real error
/// from Ollama if a name is ever stale rather than silently failing.
const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "gemma3:1b",
        label: "Gemma 3 (1B) — fastest",
        size_gb: 0.8,
    },
    CatalogEntry {
        id: "gemma3:4b",
        label: "Gemma 3 (4B) — balanced",
        size_gb: 3.3,
    },
    CatalogEntry {
        id: "llama3.2:1b",
        label: "Llama 3.2 (1B) — fastest",
        size_gb: 1.3,
    },
    CatalogEntry {
        id: "llama3.2:3b",
        label: "Llama 3.2 (3B) — balanced",
        size_gb: 2.0,
    },
    CatalogEntry {
        id: "phi3.5",
        label: "Phi-3.5 Mini (3.8B)",
        size_gb: 2.2,
    },
    CatalogEntry {
        id: "qwen2.5:3b",
        label: "Qwen 2.5 (3B)",
        size_gb: 1.9,
    },
    CatalogEntry {
        id: "mistral:7b",
        label: "Mistral (7B) — larger, more capable",
        size_gb: 4.1,
    },
];

#[derive(Serialize)]
pub struct LlmModelStatus {
    pub id: String,
    pub label: String,
    pub size_gb: f32,
    pub downloaded: bool,
    pub last_latency_ms: Option<u64>,
}

/// Catalog entries plus any already-pulled models not in the catalog
/// (e.g. whatever the user already had via `ollama pull` before this
/// feature existed), so nothing already installed becomes invisible.
// Off the main thread: sync commands run on it, and this one does I/O.
#[tauri::command(async)]
pub fn list_llm_catalog() -> Vec<LlmModelStatus> {
    let pulled = list_ollama_models();
    let mut result: Vec<LlmModelStatus> = CATALOG
        .iter()
        .map(|m| LlmModelStatus {
            id: m.id.to_string(),
            label: m.label.to_string(),
            size_gb: m.size_gb,
            downloaded: pulled.iter().any(|p| p == m.id),
            last_latency_ms: last_latency_ms(m.id),
        })
        .collect();

    for p in &pulled {
        if !result.iter().any(|r| &r.id == p) {
            result.push(LlmModelStatus {
                id: p.clone(),
                label: p.clone(),
                size_gb: 0.0,
                downloaded: true,
                last_latency_ms: last_latency_ms(p),
            });
        }
    }
    result
}

fn pull_ollama_model(app: &AppHandle, id: &str) -> Result<(), String> {
    require_local(id)?;
    let body = serde_json::json!({ "name": id, "stream": true });
    let resp = ureq::post(OLLAMA_PULL_URL)
        .timeout(std::time::Duration::from_secs(30 * 60))
        .send_json(body)
        .map_err(|e| format!("Ollama pull request failed (is `ollama serve` running?): {e}"))?;

    let reader = std::io::BufReader::new(resp.into_reader());
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
            return Err(err.to_string());
        }
        let status = parsed.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let completed = parsed.get("completed").and_then(|v| v.as_u64());
        let total = parsed.get("total").and_then(|v| v.as_u64());
        let percent = match (completed, total) {
            (Some(c), Some(t)) if t > 0 => Some(((c as f64 / t as f64) * 100.0) as u32),
            _ => None,
        };
        let _ = app.emit(
            "llm-pull-progress",
            serde_json::json!({ "id": id, "status": status, "percent": percent }),
        );
        if status == "success" {
            return Ok(());
        }
    }
    Err("Ollama closed the connection before confirming the pull finished".to_string())
}

#[tauri::command]
pub fn pull_llm_model(app: AppHandle, id: String) {
    std::thread::spawn(move || match pull_ollama_model(&app, &id) {
        Ok(()) => {
            let _ = app.emit("llm-pull-complete", &id);
        }
        Err(err) => {
            let _ = app.emit(
                "llm-pull-error",
                serde_json::json!({ "id": id, "error": err }),
            );
        }
    });
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagEntry>,
}

#[derive(Deserialize)]
struct TagEntry {
    name: String,
    /// Set by Ollama for cloud models, which it runs on its own servers.
    #[serde(default)]
    remote_host: Option<String>,
}

/// Why a cloud model is refused, shown where the request would have gone.
pub const CLOUD_REFUSED: &str =
    "Ollama cloud models are turned off in Mouthpiece so your text stays on your Mac.";

/// Ollama's cloud models ("gpt-oss:120b-cloud", "kimi-k2:cloud") send the
/// prompt to Ollama's servers. Mouthpiece promises nothing you say leaves
/// your Mac, so they're never listed or used.
pub fn is_cloud_model(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    let tag = name.rsplit_once(':').map_or("", |(_, tag)| tag);
    tag == "cloud" || tag.ends_with("-cloud") || name.ends_with("-cloud")
}

/// Refuses a cloud model before any request is made.
fn require_local(model: &str) -> Result<(), String> {
    if is_cloud_model(model) {
        crate::applog!("llm: refused cloud model {model}");
        Err(CLOUD_REFUSED.to_string())
    } else {
        Ok(())
    }
}

fn prompt_for_mode(
    mode: crate::modes::Behavior,
    text: &str,
    custom_instructions: Option<&str>,
    vocabulary: &[String],
) -> String {
    let instruction = match mode {
        // Terminal text is usually a message to a coding agent or prose,
        // not a command to synthesize. The old "rewrite as a single shell
        // command" prompt turned ordinary sentences into `ls -la` and
        // invented grep pipelines; the deterministic `format_as_cli`
        // patterns already cover the real spoken-command cases.
        crate::modes::Behavior::Cli => {
            "Clean up this dictated text: fix filler words, false starts, and obvious \
             transcription errors. Never turn it into a shell command or add anything that \
             wasn't said. Keep file names, paths, flags, code, and technical terms exactly as \
             written. Output ONLY the cleaned text, nothing else."
        }
        crate::modes::Behavior::Casual => {
            "Clean up this dictated text: fix filler words, false starts, and grammar, but keep \
             it casual and conversational — don't make it sound formal. Output ONLY the cleaned \
             text, nothing else."
        }
        crate::modes::Behavior::Plain => {
            "Clean up this dictated text: fix filler words, false starts, and obvious \
             transcription errors, but don't change its meaning or tone. Output ONLY the \
             cleaned text, nothing else."
        }
    };
    // User-supplied per-mode steering (e.g. "sign off messages with 'thanks, Jake'"),
    // folded in as its own instruction line so it can't be confused with the
    // dictated text itself. Skipped entirely when unset/blank rather than
    // appending an empty "Additional instructions:" line for every refinement.
    let mut prompt = instruction.to_string();
    // The user's vocabulary (project names, tools, jargon), so a term
    // Whisper misheard ("fire store", "cube control") comes back spelled
    // the way the user writes it.
    let terms: Vec<&str> = vocabulary
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    if !terms.is_empty() {
        prompt.push_str(&format!(
            "\n\nThe speaker uses these terms; if the text contains a misheard version of one, \
             spell it exactly like this: {}",
            terms.join(", ")
        ));
    }
    if let Some(extra) = custom_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        prompt.push_str(&format!(
            "\n\nAdditional instructions from the user: {extra}"
        ));
    }
    prompt.push_str(&format!("\n\nDictated text: {text}"));
    prompt
}

/// Sends the transcript to a locally-running Ollama instance for
/// mode-aware cleanup. Returns a clear error (rather than hanging or
/// panicking) if Ollama isn't installed/running — callers should fall back
/// to the un-refined text rather than blocking the paste on this.
pub fn refine(
    mode: crate::modes::Behavior,
    text: &str,
    model: &str,
    custom_instructions: Option<&str>,
) -> Result<String, String> {
    let never = std::sync::atomic::AtomicBool::new(false);
    refine_streaming(
        RefineRequest {
            mode,
            text,
            model,
            custom_instructions,
            vocabulary: &[],
        },
        &|_| {},
        &never,
    )
}

pub struct RefineRequest<'a> {
    pub mode: crate::modes::Behavior,
    pub text: &'a str,
    pub model: &'a str,
    pub custom_instructions: Option<&'a str>,
    pub vocabulary: &'a [String],
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
}

enum StreamEvent {
    Text(String),
    Done,
    Failed(String),
}

/// `refine`, streamed: Ollama sends the answer as it writes it, which costs
/// nothing extra and gives three things the one-shot request couldn't:
/// - exact progress (`on_progress`, 0-99): a cleanup is about as long as
///   its input, so characters written over characters in is the real
///   fraction done;
/// - an early stop when the answer starts with chatter ("Sure! Here's…")
///   or runs far longer than the input, instead of waiting for the whole
///   bad answer before rejecting it;
/// - instant cancel: `cancel` (the user's "paste as-is") returns right
///   away, even while Ollama is still processing the prompt.
pub fn refine_streaming(
    req: RefineRequest,
    on_progress: &dyn Fn(u8),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String, String> {
    refine_streaming_at(OLLAMA_GENERATE_URL, req, on_progress, cancel)
}

fn refine_streaming_at(
    url: &'static str,
    req: RefineRequest,
    on_progress: &dyn Fn(u8),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    require_local(req.model)?;

    let body = serde_json::json!({
        "model": req.model,
        "prompt": prompt_for_mode(req.mode, req.text, req.custom_instructions, req.vocabulary),
        "stream": true,
        "keep_alive": KEEP_ALIVE,
        // Dictation cleanup needs to be fast, not deliberate — reasoning
        // ("thinking") models otherwise spend many seconds on an internal
        // chain-of-thought before ever answering. Ignored by models that
        // don't support it.
        "think": false,
    });

    // The HTTP read happens on its own thread so a cancel never waits on
    // it; `stop` tells that thread to hang up, which makes Ollama stop
    // generating.
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader_stop = stop.clone();
    std::thread::spawn(move || {
        let resp = match ureq::post(url)
            .timeout(std::time::Duration::from_secs(30))
            .send_json(body)
        {
            Ok(resp) => resp,
            Err(e) => {
                let _ = tx.send(StreamEvent::Failed(format!(
                    "Ollama request failed (is `ollama serve` running?): {e}"
                )));
                return;
            }
        };
        for line in std::io::BufReader::new(resp.into_reader()).lines() {
            if reader_stop.load(Ordering::Relaxed) {
                return;
            }
            let event = match line {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => match serde_json::from_str::<StreamChunk>(&line) {
                    Ok(chunk) if chunk.done => {
                        let _ = tx.send(StreamEvent::Text(chunk.response));
                        StreamEvent::Done
                    }
                    Ok(chunk) => StreamEvent::Text(chunk.response),
                    Err(e) => StreamEvent::Failed(format!("failed to parse Ollama response: {e}")),
                },
                Err(e) => StreamEvent::Failed(format!("Ollama stream broke: {e}")),
            };
            let finished = !matches!(event, StreamEvent::Text(_));
            if tx.send(event).is_err() || finished {
                return;
            }
        }
        let _ = tx.send(StreamEvent::Failed("Ollama closed the stream early".into()));
    });

    let started = Instant::now();
    let input_chars = req.text.chars().count().max(1);
    let mut output = String::new();
    let mut last_percent = None;
    let hang_up = |reason: String| {
        stop.store(true, Ordering::Relaxed);
        Err(reason)
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            return hang_up("skipped by the user".into());
        }
        match rx.recv_timeout(std::time::Duration::from_millis(40)) {
            Ok(StreamEvent::Text(piece)) => {
                output.push_str(&piece);
                let written = output.chars().count();
                let percent = (written * 100 / input_chars).min(99) as u8;
                if last_percent != Some(percent) {
                    on_progress(percent);
                    last_percent = Some(percent);
                }
                if (24..48).contains(&written) {
                    if let Some(reason) = early_rejection(&output) {
                        return hang_up(format!("stopped early ({reason})"));
                    }
                }
                if written > input_chars * 2 + 120 {
                    return hang_up(
                        "stopped early (answer ran far longer than the dictation)".into(),
                    );
                }
            }
            Ok(StreamEvent::Done) => break,
            Ok(StreamEvent::Failed(err)) => return Err(err),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Ollama stream ended unexpectedly".into())
            }
        }
    }

    // Only recorded on a successful round-trip — a failed/timed-out request
    // isn't a meaningful "how fast is this model" data point.
    record_latency(req.model, started.elapsed().as_millis() as u64);

    // Small models sometimes wrap plain prose in a fenced block even
    // though none of the prompts ask for code (observed via the eval
    // suite: "make directory for the new service" came back as
    // "```bash\nmkdir -p\n```").
    let cleaned = strip_markdown_fences(output.trim());
    if cleaned.is_empty() {
        Err("Ollama returned an empty response".to_string())
    } else {
        Ok(cleaned)
    }
}

/// The start of an answer that's already known to be unusable, so the
/// stream can be cut off instead of generated in full.
fn early_rejection(prefix: &str) -> Option<&'static str> {
    let lower = prefix.trim_start().to_lowercase();
    if PREAMBLES.iter().any(|m| lower.starts_with(m)) {
        Some("conversational preamble")
    } else if lower.starts_with("```") {
        Some("markdown fences")
    } else {
        None
    }
}

const PREAMBLES: &[&str] = &[
    "sure,",
    "sure!",
    "here's",
    "here is",
    "certainly",
    "of course",
];

/// Why a refinement result shouldn't be used, if it looks broken rather
/// than cleaned: a chat-style preamble, leftover fences, words glued into
/// one blob, or most of the message gone. Small local models do all of
/// these occasionally; falling back to the unrefined (already formatted)
/// text is always safer than pasting them.
pub fn degenerate_output(input: &str, output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Some("empty output".to_string());
    }
    if trimmed.contains("```") {
        return Some("markdown fences".to_string());
    }
    let lower = trimmed.to_lowercase();
    if PREAMBLES.iter().any(|m| lower.starts_with(m)) {
        return Some("conversational preamble".to_string());
    }

    let input_words = input.split_whitespace().count();
    let output_words = trimmed.split_whitespace().count();
    if input_words >= 4 && trimmed.split_whitespace().any(|w| w.chars().count() > 30) {
        return Some("words glued together".to_string());
    }
    if input_words >= 8 && output_words < input_words / 3 {
        return Some(format!(
            "dropped from {input_words} to {output_words} words"
        ));
    }
    if input_words >= 4 && output_words > input_words * 2 + 10 {
        return Some(format!("grew from {input_words} to {output_words} words"));
    }
    None
}

/// Below this word count, a dictation is already about as short as a
/// one-line summary would be — skip the LLM call entirely rather than
/// paying for a round-trip to produce "Said hello" for a two-word message.
const JOURNAL_MIN_WORDS: usize = 6;

/// Returns `None` (not an error) when the text is too short to be worth
/// summarizing — distinct from `Some(Err(_))`, an actual failed attempt.
pub fn summarize_for_journal(text: &str, model: &str) -> Option<Result<String, String>> {
    if text.split_whitespace().count() < JOURNAL_MIN_WORDS {
        return None;
    }

    let instruction = "Summarize the following dictated text as a single short line for a work \
                        journal, in the style of a git commit subject: imperative mood, no \
                        trailing period, under 10 words. Output ONLY the summary line, nothing \
                        else.";
    if let Err(err) = require_local(model) {
        return Some(Err(err));
    }
    let body = serde_json::json!({
        "model": model,
        "prompt": format!("{instruction}\n\nDictated text: {text}"),
        "stream": false,
        "keep_alive": KEEP_ALIVE,
        "think": false,
    });

    let started = Instant::now();
    let resp = match ureq::post(OLLAMA_GENERATE_URL)
        .timeout(std::time::Duration::from_secs(30))
        .send_json(body)
    {
        Ok(resp) => resp,
        Err(e) => {
            return Some(Err(format!(
                "Ollama request failed (is `ollama serve` running?): {e}"
            )))
        }
    };

    let parsed: GenerateResponse = match resp.into_json() {
        Ok(parsed) => parsed,
        Err(e) => return Some(Err(format!("failed to parse Ollama response: {e}"))),
    };

    record_latency(model, started.elapsed().as_millis() as u64);

    let summary = strip_markdown_fences(parsed.response.trim())
        .trim_matches('"')
        .to_string();
    if summary.is_empty() {
        Some(Err("Ollama returned an empty response".to_string()))
    } else {
        Some(Ok(summary))
    }
}

/// Small models often ignore "no markdown fences" instructions and wrap
/// the response in a fenced code block anyway — strip one leading/trailing
/// fence if present so the output pastes as raw code, not fenced markdown.
fn strip_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let Some(after_open) = trimmed.find('\n').map(|idx| &trimmed[idx + 1..]) else {
        return trimmed.to_string();
    };
    match after_open.rfind("```") {
        Some(idx) => after_open[..idx].trim_end().to_string(),
        None => after_open.trim_end().to_string(),
    }
}

/// Sends a natural-language "generate boilerplate for X" request (see
/// boilerplate.rs) to Ollama for code generation. Separate from `refine()`
/// since code generation is a heavier task than dictation cleanup and
/// warrants a longer timeout.
pub fn generate_boilerplate(request: &str, model: &str) -> Result<String, String> {
    let instruction = "Generate the code requested below. Output ONLY the code itself — no \
                        explanation, no markdown code fences, no surrounding commentary. If the \
                        request doesn't specify a language, infer the most likely one from \
                        context.";
    require_local(model)?;
    let body = serde_json::json!({
        "model": model,
        "prompt": format!("{instruction}\n\nRequest: {request}"),
        "stream": false,
        "keep_alive": KEEP_ALIVE,
        "think": false,
    });

    let started = Instant::now();
    let resp = ureq::post(OLLAMA_GENERATE_URL)
        // Code generation is a heavier task than dictation cleanup and can
        // legitimately take longer than refine()'s 30s budget.
        .timeout(std::time::Duration::from_secs(60))
        .send_json(body)
        .map_err(|e| format!("Ollama request failed (is `ollama serve` running?): {e}"))?;

    let parsed: GenerateResponse = resp
        .into_json()
        .map_err(|e| format!("failed to parse Ollama response: {e}"))?;

    record_latency(model, started.elapsed().as_millis() as u64);

    let code = strip_markdown_fences(&parsed.response);
    if code.is_empty() {
        Err("Ollama returned an empty response".to_string())
    } else {
        Ok(code)
    }
}

/// Names of locally-pulled Ollama models, for the Settings picker. Empty
/// (not an error) if Ollama isn't reachable, so the UI can show "Ollama
/// not running" rather than a raw error.
// Off the main thread: sync commands run on it, and this one does I/O.
#[tauri::command(async)]
pub fn list_ollama_models() -> Vec<String> {
    ollama_models().unwrap_or_default()
}

/// Models Ollama has pulled, or `None` if Ollama isn't reachable (as
/// opposed to reachable with nothing pulled).
pub fn ollama_models() -> Option<Vec<String>> {
    // Ollama answers this in milliseconds when it's up; anything slower
    // means it's wedged, and Settings shouldn't wait on it.
    let Ok(resp) = ureq::get(OLLAMA_TAGS_URL)
        .timeout(std::time::Duration::from_secs(3))
        .call()
    else {
        return None;
    };
    let Ok(parsed) = resp.into_json::<TagsResponse>() else {
        return None;
    };
    Some(
        parsed
            .models
            .into_iter()
            .filter(|m| m.remote_host.is_none() && !is_cloud_model(&m.name))
            .map(|m| m.name)
            .collect(),
    )
}

#[tauri::command]
pub fn get_llm_only_when_needed(app: tauri::AppHandle) -> bool {
    crate::config::load(&app).llm_only_when_needed
}

#[tauri::command]
pub fn set_llm_only_when_needed(app: tauri::AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.llm_only_when_needed = enabled;
    let _ = crate::config::save(&app, &cfg);
}

#[tauri::command]
pub fn get_llm_model(app: tauri::AppHandle) -> String {
    crate::config::load(&app).llm_model
}

#[tauri::command]
pub fn set_llm_model(app: tauri::AppHandle, model: String) {
    let mut cfg = crate::config::load(&app);
    cfg.llm_model = model;
    let _ = crate::config::save(&app, &cfg);
}

#[cfg(test)]
mod tests {

    /// A one-request fake Ollama that streams `chunks` as NDJSON, pausing
    /// `gap` between them. Returns its URL (leaked to `'static`, test-only).
    fn fake_ollama(chunks: Vec<&'static str>, gap: std::time::Duration) -> &'static str {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/generate", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 16384];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n");
            let last = chunks.len() - 1;
            for (i, piece) in chunks.into_iter().enumerate() {
                let line = serde_json::json!({ "response": piece, "done": i == last }).to_string();
                if stream.write_all(format!("{line}\n").as_bytes()).is_err() {
                    return;
                }
                let _ = stream.flush();
                std::thread::sleep(gap);
            }
        });
        Box::leak(url.into_boxed_str())
    }

    fn req(text: &'static str) -> RefineRequest<'static> {
        RefineRequest {
            mode: crate::modes::Behavior::Plain,
            text,
            model: "fake",
            custom_instructions: None,
            vocabulary: &[],
        }
    }

    #[test]
    fn streaming_reports_exact_progress_and_assembles_the_text() {
        let url = fake_ollama(
            vec!["Ship ", "the release ", "tonight."],
            std::time::Duration::from_millis(20),
        );
        let seen = std::sync::Mutex::new(Vec::new());
        let never = std::sync::atomic::AtomicBool::new(false);
        let out = refine_streaming_at(
            url,
            req("ship the release tonight"),
            &|p| seen.lock().unwrap().push(p),
            &never,
        );
        assert_eq!(out.unwrap(), "Ship the release tonight.");
        let seen = seen.into_inner().unwrap();
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert!(seen.last().is_some_and(|&p| p > 80 && p <= 99), "{seen:?}");
    }

    #[test]
    fn streaming_stops_early_on_a_chatty_answer() {
        let url = fake_ollama(
            vec![
                "Sure! Here's the cleaned-up text: ",
                "Ship the release tonight.",
                " Let me know!",
            ],
            std::time::Duration::from_millis(300),
        );
        let never = std::sync::atomic::AtomicBool::new(false);
        let started = std::time::Instant::now();
        let out = refine_streaming_at(url, req("ship the release tonight"), &|_| {}, &never);
        assert!(out.unwrap_err().contains("conversational preamble"));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "didn't stop early"
        );
    }

    #[test]
    fn cancel_returns_immediately() {
        let url = fake_ollama(vec!["slow", " answer"], std::time::Duration::from_secs(5));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let out = refine_streaming_at(url, req("a slow answer"), &|_| {}, &cancel);
            assert_eq!(out.unwrap_err(), "skipped by the user");
        });
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn prompt_includes_the_vocabulary() {
        let vocab = vec!["Firestore".to_string(), "kubectl".to_string()];
        let prompt = prompt_for_mode(crate::modes::Behavior::Cli, "fire store", None, &vocab);
        assert!(prompt.contains("spell it exactly like this: Firestore, kubectl"));
        assert!(prompt.ends_with("Dictated text: fire store"));
        assert!(!prompt_for_mode(crate::modes::Behavior::Cli, "x", None, &[]).contains("spell it"));
    }
    use super::*;

    #[test]
    fn cloud_models_are_recognized() {
        for cloud in [
            "gpt-oss:120b-cloud",
            "qwen3-coder:480b-cloud",
            "kimi-k2:cloud",
            "GLM-4.6:Cloud",
        ] {
            assert!(is_cloud_model(cloud), "{cloud}");
        }
        for local in [
            "gemma3:4b",
            "llama3.2:3b",
            "qwen2.5:1.5b-instruct",
            "cloudy-model:7b",
        ] {
            assert!(!is_cloud_model(local), "{local}");
        }
        assert_eq!(
            require_local("gpt-oss:20b-cloud"),
            Err(CLOUD_REFUSED.to_string())
        );
    }

    /// Smoke test against a real, locally-running Ollama instance — skips
    /// (doesn't fail) if Ollama isn't reachable, e.g. in CI or on a machine
    /// that hasn't installed it. Verifies the actual HTTP/JSON integration,
    /// not just that the code compiles.
    #[test]
    fn refines_text_via_real_ollama() {
        let models = list_ollama_models();
        if models.is_empty() {
            eprintln!("skipping: no Ollama models found (is `ollama serve` running?)");
            return;
        }
        let model = &models[0];

        let result = refine(
            crate::modes::Behavior::Cli,
            "git commit update the readme file",
            model,
            None,
        );

        match result {
            Ok(text) => {
                assert!(!text.is_empty(), "refined text should not be empty");
                eprintln!("Ollama ({model}) refined CLI text to: {text:?}");
            }
            Err(err) => panic!("Ollama refinement failed: {err}"),
        }

        let latency = last_latency_ms(model);
        assert!(
            latency.is_some(),
            "successful refine() should record a latency"
        );
        eprintln!("Ollama ({model}) refine() latency: {}ms", latency.unwrap());
    }

    #[test]
    fn prompt_for_mode_folds_in_custom_instructions_when_present() {
        let prompt = prompt_for_mode(
            crate::modes::Behavior::Plain,
            "hello",
            Some("be brief"),
            &[],
        );
        assert!(prompt.contains("Additional instructions from the user: be brief"));
    }

    #[test]
    fn prompt_for_mode_omits_custom_instructions_block_when_blank_or_absent() {
        let without = prompt_for_mode(crate::modes::Behavior::Plain, "hello", None, &[]);
        assert!(!without.contains("Additional instructions"));
        let blank = prompt_for_mode(crate::modes::Behavior::Plain, "hello", Some("   "), &[]);
        assert!(!blank.contains("Additional instructions"));
    }

    #[test]
    fn strip_markdown_fences_removes_fenced_block_with_language_tag() {
        let input = "```python\ndef add(a, b):\n    return a + b\n```";
        assert_eq!(
            strip_markdown_fences(input),
            "def add(a, b):\n    return a + b"
        );
    }

    #[test]
    fn strip_markdown_fences_removes_fenced_block_without_language_tag() {
        let input = "```\nlet x = 1;\n```";
        assert_eq!(strip_markdown_fences(input), "let x = 1;");
    }

    #[test]
    fn strip_markdown_fences_leaves_unfenced_text_unchanged() {
        assert_eq!(strip_markdown_fences("let x = 1;"), "let x = 1;");
    }

    /// Smoke test against a real, locally-running Ollama instance — skips
    /// if Ollama isn't reachable, matching `refines_text_via_real_ollama`.
    #[test]
    fn generates_boilerplate_via_real_ollama() {
        let models = list_ollama_models();
        if models.is_empty() {
            eprintln!("skipping: no Ollama models found (is `ollama serve` running?)");
            return;
        }
        let model = &models[0];

        let result = generate_boilerplate("a Python function that adds two numbers", model);

        match result {
            Ok(code) => {
                assert!(!code.is_empty(), "generated code should not be empty");
                assert!(
                    !code.trim_start().starts_with("```"),
                    "output should have markdown fences stripped, got: {code:?}"
                );
                eprintln!("Ollama ({model}) generated boilerplate: {code:?}");
            }
            Err(err) => panic!("boilerplate generation failed: {err}"),
        }
    }

    /// Representative dictations per mode, picked from real manual-testing
    /// reports rather than invented examples. The CLI prose case is the
    /// exact sentence the old "rewrite as a shell command" prompt turned
    /// into a bogus command. Add to this as new issues turn up.
    const EVAL_CASES: &[(crate::modes::Behavior, &str)] = &[
        (
            crate::modes::Behavior::Cli,
            "Let's test and see what happens when we try and paste in here. If it truly is just a \
             problem with this terminal, we still need to work it out.",
        ),
        (
            crate::modes::Behavior::Cli,
            "um look at recording.rs and uh fix the paste function",
        ),
        (
            crate::modes::Behavior::Plain,
            "um so I think we should probably go with the the second approach because it's \
             it's simpler and also easier to test",
        ),
        (
            crate::modes::Behavior::Casual,
            "hey can you like send that file over when you get a sec no rush at all",
        ),
    ];

    /// Runs `EVAL_CASES` against a real, locally-running Ollama instance,
    /// checking each output with the same `degenerate_output` guard the
    /// real pipeline uses, plus: CLI output must never look like a shell
    /// command synthesized from prose, and file names must survive.
    /// Not a strict correctness check (LLM output isn't deterministic);
    /// skips if Ollama isn't reachable, like the other live tests here.
    /// Set `DW_EVAL_MODEL=qwen3.5:4b` (or whatever Settings uses) to eval
    /// the model you actually dictate with instead of Ollama's first one.
    #[test]
    fn eval_suite_against_real_ollama() {
        let models = list_ollama_models();
        if models.is_empty() {
            eprintln!("skipping: no Ollama models found (is `ollama serve` running?)");
            return;
        }
        let model = &std::env::var("DW_EVAL_MODEL").unwrap_or_else(|_| models[0].clone());

        let mut failures = Vec::new();
        for &(mode, input) in EVAL_CASES {
            // Mirrors the real pipeline: `refine()` only ever sees
            // `apply_mode`'s output, never the raw transcript.
            let pre_formatted = crate::modes::apply_mode(mode, input);
            match refine(mode, &pre_formatted, model, None) {
                Ok(output) => {
                    eprintln!("[{mode:?}] {input:?} -> {output:?}");
                    if let Some(reason) = degenerate_output(&pre_formatted, &output) {
                        failures.push(format!("{mode:?} {input:?}: {reason}: {output:?}"));
                    }
                    let first = output
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_lowercase();
                    if mode == crate::modes::Behavior::Cli
                        && ["ls", "grep", "cd", "echo", "cat", "find", "awk", "sed"]
                            .contains(&first.as_str())
                        && !pre_formatted.to_lowercase().starts_with(&first)
                    {
                        failures.push(format!(
                            "{mode:?} {input:?}: turned prose into a command: {output:?}"
                        ));
                    }
                    if pre_formatted.contains("recording.rs") && !output.contains("recording.rs") {
                        failures.push(format!(
                            "{mode:?} {input:?}: lost the file reference: {output:?}"
                        ));
                    }
                }
                Err(err) => failures.push(format!("{mode:?} {input:?}: refine() failed: {err}")),
            }
        }

        assert!(
            failures.is_empty(),
            "eval suite failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn degenerate_output_flags_real_failure_shapes() {
        let prose = "Let's test and see what happens when we try and paste in here";
        assert!(degenerate_output(
            prose,
            "test_paste_what_happens_truly_problem_terminal_work_out"
        )
        .is_some());
        assert!(degenerate_output(prose, "```bash\nls -la\n```").is_some());
        assert!(degenerate_output(prose, "Sure, here's the cleaned text: ...").is_some());
        assert!(degenerate_output(prose, "ls -la").is_some());
        assert!(degenerate_output(prose, "").is_some());
    }

    #[test]
    fn degenerate_output_accepts_a_normal_cleanup() {
        assert_eq!(
            degenerate_output(
                "um so I think we should probably go with the the second approach",
                "I think we should probably go with the second approach."
            ),
            None
        );
        assert_eq!(degenerate_output("fix bug", "Fix the bug."), None);
    }

    #[test]
    fn summarize_for_journal_skips_short_dictations() {
        assert!(summarize_for_journal("fix the bug", "any-model").is_none());
        assert!(summarize_for_journal("git commit", "any-model").is_none());
    }

    /// Smoke test against a real, locally-running Ollama instance — skips
    /// if Ollama isn't reachable, matching the other live tests in this
    /// file.
    #[test]
    fn summarizes_a_real_dictation_via_real_ollama() {
        let models = list_ollama_models();
        if models.is_empty() {
            eprintln!("skipping: no Ollama models found (is `ollama serve` running?)");
            return;
        }
        let model = &models[0];

        let result = summarize_for_journal(
            "okay so I need to go back and fix the transcribe and paste function because it's \
             not handling the case where the model override is set but the path doesn't resolve",
            model,
        );

        match result {
            Some(Ok(summary)) => {
                assert!(!summary.is_empty(), "summary should not be empty");
                eprintln!("Ollama ({model}) journal summary: {summary:?}");
            }
            Some(Err(err)) => panic!("journal summarization failed: {err}"),
            None => panic!("dictation was long enough that it should not have been skipped"),
        }
    }
}
