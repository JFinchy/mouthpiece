use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

/// A slow or unreachable endpoint should never add latency to the paste —
/// short enough that a hung endpoint doesn't pile up background threads
/// across several dictations, generous enough for a real webhook consumer
/// (n8n/Zapier/Make.com) to respond.
const WEBHOOK_TIMEOUT_SECS: u64 = 10;

#[derive(Serialize)]
pub struct WebhookPayload {
    pub timestamp_ms: u64,
    pub text: String,
    /// Known v1 gap: journal summaries are generated asynchronously after
    /// the webhook fires, so this is `None` whenever `journal_enabled` is
    /// on — see BACKLOG.md.
    pub summary: Option<String>,
    pub app_name: Option<String>,
    pub mode: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn post(url: &str, payload: &WebhookPayload) -> Result<(), String> {
    ureq::post(url)
        .timeout(std::time::Duration::from_secs(WEBHOOK_TIMEOUT_SECS))
        .send_json(serde_json::to_value(payload).map_err(|e| e.to_string())?)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Fires in its own background thread (same pattern as journal
/// summarization in `recording.rs`). Failures go through `applog!`, never
/// surfaced to the user — a webhook is a side channel, not the primary
/// delivery path, and the transcript has already been pasted successfully
/// by the time this runs.
pub fn send_entry(url: String, payload: WebhookPayload) {
    std::thread::spawn(move || {
        if let Err(err) = post(&url, &payload) {
            crate::applog!("webhook: delivery to {url} failed: {err}");
        }
    });
}

#[tauri::command]
pub fn get_webhook_url(app: AppHandle) -> Option<String> {
    crate::config::load(&app).webhook_url
}

#[tauri::command]
pub fn set_webhook_url(app: AppHandle, url: Option<String>) {
    let mut cfg = crate::config::load(&app);
    cfg.webhook_url = url.filter(|u| !u.trim().is_empty());
    let _ = crate::config::save(&app, &cfg);
}

/// Lets a user confirm their Zapier/n8n/webhook.site endpoint is wired
/// correctly without waiting for a real dictation. Unlike `send_entry`,
/// failures here *are* surfaced (via `webhook-test-result`) — the whole
/// point of a test button is telling the user whether it worked.
#[tauri::command]
pub fn send_test_webhook(app: AppHandle) -> Result<(), String> {
    let url = crate::config::load(&app)
        .webhook_url
        .ok_or_else(|| "no webhook URL configured".to_string())?;

    std::thread::spawn(move || {
        let payload = WebhookPayload {
            timestamp_ms: now_ms(),
            text: "This is a test event from Mouthpiece.".to_string(),
            summary: None,
            app_name: None,
            mode: Some("test".to_string()),
        };
        match post(&url, &payload) {
            Ok(()) => {
                let _ = app.emit("webhook-test-result", serde_json::json!({ "ok": true }));
            }
            Err(err) => {
                crate::applog!("webhook: test send to {url} failed: {err}");
                let _ = app.emit(
                    "webhook-test-result",
                    serde_json::json!({ "ok": false, "error": err }),
                );
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A one-shot local HTTP server: accepts a single connection, captures
    /// its request body, and replies 200 OK. Runs against a real socket
    /// rather than mocking `ureq`, so this actually exercises `post()`'s
    /// wire format — but stays hermetic (no external service, no Ollama-
    /// style "skip if unreachable") since it's our own listener.
    fn spawn_mock_server() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // Read until the whole body has arrived: the client may send
            // headers and body in separate writes, and one `read` would
            // then see an empty body (an intermittent test failure).
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            let body = loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let expected = head
                        .lines()
                        .find_map(|l| {
                            let (name, value) = l.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if body.len() >= expected || n == 0 {
                        break body.to_string();
                    }
                } else if n == 0 {
                    break String::new();
                }
            };
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            let _ = tx.send(body);
        });
        (format!("http://{addr}"), rx)
    }

    #[test]
    fn post_sends_the_payload_fields_as_a_json_body() {
        let (url, rx) = spawn_mock_server();
        let payload = WebhookPayload {
            timestamp_ms: 1_700_000_000_000,
            text: "git commit update the readme".to_string(),
            summary: None,
            app_name: Some("Terminal".to_string()),
            mode: Some("cli".to_string()),
        };

        post(&url, &payload).expect("post to local mock server should succeed");

        let body = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("mock server should have received a request");
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("body should be valid JSON");
        assert_eq!(parsed["timestamp_ms"], 1_700_000_000_000_u64);
        assert_eq!(parsed["text"], "git commit update the readme");
        assert_eq!(parsed["app_name"], "Terminal");
        assert_eq!(parsed["mode"], "cli");
        assert!(parsed["summary"].is_null());
    }

    #[test]
    fn post_returns_err_when_the_endpoint_is_unreachable() {
        // Port 1 (TCPMUX) is a reserved port nothing binds to in test
        // environments — connection refused is immediate and deterministic,
        // unlike a real timeout-based unreachable-host test.
        let payload = WebhookPayload {
            timestamp_ms: 0,
            text: "x".to_string(),
            summary: None,
            app_name: None,
            mode: None,
        };
        let result = post("http://127.0.0.1:1", &payload);
        assert!(result.is_err());
    }
}
