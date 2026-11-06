//! The `mouthpiece` command, and its MCP server for coding agents.
//!
//! The app binary doubles as the command: Mouthpiece keeps a small script
//! at `~/Library/Application Support/dev.mouthpiece/bin/mouthpiece` that
//! runs it with `--mouthpiece-cli`, and `main` hands those runs here
//! before any window or Tauri code starts. Everything goes through the
//! running app's socket (`cli_server.rs`), so the command does nothing
//! when Mouthpiece isn't open.
//!
//! ```text
//! mouthpiece listen [--prompt TEXT] [--timeout SECONDS]
//! mouthpiece status
//! mouthpiece vocab add TERM...
//! mouthpiece vocab list
//! mouthpiece mcp
//! ```

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub const FLAG: &str = "--mouthpiece-cli";
const APP_ID: &str = "dev.mouthpiece";
const MCP_PROTOCOL: &str = "2025-06-18";

const USAGE: &str = "Usage: mouthpiece <command>

  listen [--prompt TEXT] [--timeout SECONDS]
                     Record one dictation and print its text instead of
                     pasting it. Stop it with your dictation shortcut.
  status             Show what Mouthpiece is doing, as JSON.
  vocab add TERM...  Add words to the vocabulary.
  vocab list         Print the vocabulary, one term per line.
  mcp                Run as an MCP server on stdin/stdout, for coding agents.

Mouthpiece has to be running.";

/// Runs the command if this process was started as one. `main` calls this
/// first; `None` means start the app as usual.
pub fn run_if_cli() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(FLAG) {
        return None;
    }
    let rest: Vec<String> = args.collect();
    Some(run(&rest))
}

fn socket_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library/Application Support")
            .join(APP_ID)
            .join(crate::cli_server::SOCKET_NAME),
    )
}

/// Sends one request to the running app and returns its reply.
fn request(req: &Value, wait: Duration) -> Result<Value, String> {
    let path = socket_path().ok_or("HOME isn't set")?;
    request_at(&path, req, wait)
}

fn request_at(path: &std::path::Path, req: &Value, wait: Duration) -> Result<Value, String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|_| "Mouthpiece isn't running. Open it and try again.".to_string())?;
    stream
        .set_read_timeout(Some(wait))
        .map_err(|e| e.to_string())?;
    writeln!(stream, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| format!("no reply from Mouthpiece: {e}"))?;
    let reply: Value =
        serde_json::from_str(&line).map_err(|e| format!("bad reply from Mouthpiece: {e}"))?;
    if reply["ok"].as_bool() == Some(true) {
        Ok(reply)
    } else {
        Err(reply["error"]
            .as_str()
            .unwrap_or("Mouthpiece refused")
            .to_string())
    }
}

fn listen(prompt: Option<&str>, timeout_s: Option<u64>) -> Result<String, String> {
    let timeout = timeout_s.unwrap_or(180);
    let reply = request(
        &json!({"cmd": "listen", "prompt": prompt, "timeout_s": timeout}),
        // Room for transcribing and cleanup after the recording stops.
        Duration::from_secs(timeout + 120),
    )?;
    Ok(reply["text"].as_str().unwrap_or_default().to_string())
}

fn vocab_add(terms: &[String]) -> Result<Vec<String>, String> {
    let reply = request(
        &json!({"cmd": "vocab_add", "terms": terms}),
        Duration::from_secs(10),
    )?;
    Ok(reply["added"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

fn run(args: &[String]) -> i32 {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let result: Result<(), String> = match words.as_slice() {
        ["listen", rest @ ..] => match parse_listen(rest) {
            Ok((prompt, timeout)) => {
                listen(prompt.as_deref(), timeout).map(|text| println!("{text}"))
            }
            Err(err) => {
                eprintln!("{err}\n\n{USAGE}");
                return 2;
            }
        },
        ["status"] => request(&json!({"cmd": "status"}), Duration::from_secs(10)).map(|r| {
            println!(
                "{}",
                serde_json::to_string_pretty(&r["status"]).unwrap_or_default()
            )
        }),
        ["vocab", "add", terms @ ..] if !terms.is_empty() => {
            let terms: Vec<String> = terms.iter().map(|t| t.to_string()).collect();
            vocab_add(&terms).map(|added| {
                if added.is_empty() {
                    println!("Already in the vocabulary.");
                } else {
                    println!("Added: {}", added.join(", "));
                }
            })
        }
        ["vocab", "list"] => {
            request(&json!({"cmd": "vocab_list"}), Duration::from_secs(10)).map(|r| {
                for term in r["terms"].as_array().into_iter().flatten() {
                    if let Some(t) = term.as_str() {
                        println!("{t}");
                    }
                }
            })
        }
        ["mcp"] => {
            mcp_serve(std::io::stdin().lock(), std::io::stdout().lock());
            Ok(())
        }
        [] | ["help"] | ["--help"] | ["-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("mouthpiece: {err}");
            1
        }
    }
}

fn parse_listen(args: &[&str]) -> Result<(Option<String>, Option<u64>), String> {
    let (mut prompt, mut timeout) = (None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--prompt" | "-p" => prompt = Some(it.next().ok_or("--prompt needs text")?.to_string()),
            "--timeout" | "-t" => {
                let v = it.next().ok_or("--timeout needs a number of seconds")?;
                timeout = Some(
                    v.parse()
                        .map_err(|_| format!("--timeout: {v:?} isn't a number"))?,
                );
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok((prompt, timeout))
}

// ----------------------------------------------------------------- MCP

fn tools() -> Value {
    json!([
        {
            "name": "ask_user",
            "description": "Ask the user a question and get their spoken answer, transcribed on their Mac by Mouthpiece. \
    The question appears in Mouthpiece's widget; the user answers out loud and stops the recording with their dictation \
    shortcut. Use it when you need a decision or detail and the user may be away from the keyboard. Blocks until they \
    answer, delete the recording, or the timeout passes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "Short question shown to the user (up to 200 characters)."},
                    "timeout_seconds": {"type": "integer", "minimum": 5, "maximum": 900, "description": "How long to wait for an answer. Default 180."}
                },
                "required": ["question"]
            }
        },
        {
            "name": "add_vocabulary",
            "description": "Teach Mouthpiece's speech recognition words it should know: project names, libraries, \
    acronyms, people. Useful after you introduce a new name the user will say out loud.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "terms": {"type": "array", "items": {"type": "string"}, "maxItems": 50}
                },
                "required": ["terms"]
            }
        },
        {
            "name": "mouthpiece_status",
            "description": "Whether Mouthpiece is idle or recording, its version, and which coding agents are waiting for the user.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn call_tool(name: &str, args: &Value) -> Value {
    match name {
        "ask_user" => {
            let Some(question) = args["question"].as_str() else {
                return text_result("ask_user needs a question".into(), true);
            };
            let timeout = args["timeout_seconds"].as_u64();
            match listen(Some(question), timeout) {
                Ok(text) => text_result(text, false),
                Err(err) => text_result(err, true),
            }
        }
        "add_vocabulary" => {
            let terms: Vec<String> = args["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            match vocab_add(&terms) {
                Ok(added) if added.is_empty() => {
                    text_result("All of those were already in the vocabulary.".into(), false)
                }
                Ok(added) => text_result(format!("Added: {}", added.join(", ")), false),
                Err(err) => text_result(err, true),
            }
        }
        "mouthpiece_status" => match request(&json!({"cmd": "status"}), Duration::from_secs(10)) {
            Ok(r) => text_result(r["status"].to_string(), false),
            Err(err) => text_result(err, true),
        },
        other => text_result(format!("unknown tool {other}"), true),
    }
}

/// Answers one JSON-RPC message; `None` for notifications.
pub fn mcp_handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(MCP_PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mouthpiece", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Mouthpiece is the user's local dictation app. Use ask_user to ask the user something out loud \
        when they may be away from the keyboard, and add_vocabulary to teach it names the user will say."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let empty = json!({});
            let args = msg["params"].get("arguments").unwrap_or(&empty);
            Ok(call_tool(name, args))
        }
        _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    })
}

/// Newline-delimited JSON-RPC over stdio, one message per line.
pub fn mcp_serve(input: impl BufRead, mut output: impl Write) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => mcp_handle(&msg),
            Err(err) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": err.to_string()}}),
            ),
        };
        if let Some(reply) = reply {
            if writeln!(output, "{reply}")
                .and_then(|()| output.flush())
                .is_err()
            {
                break;
            }
        }
    }
}

/// The script that makes `mouthpiece` a command, pointing at this binary.
pub fn wrapper_script(exe: &std::path::Path) -> String {
    let quoted = exe.to_string_lossy().replace('\'', r"'\''");
    format!(
        "#!/bin/sh\n# The `mouthpiece` command. Mouthpiece rewrites this file at launch so it\n\
         # points at the copy of the app that's installed.\nexec '{quoted}' {FLAG} \"$@\"\n"
    )
}

/// Keeps `bin/mouthpiece` in the app data folder pointing at this binary.
pub fn install_wrapper(data_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = data_dir.join("bin").join("mouthpiece");
    let script = wrapper_script(&exe);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, &script).map_err(|e| e.to_string())?;
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_options_parse() {
        assert_eq!(
            parse_listen(&["--prompt", "Which branch?", "-t", "30"]).unwrap(),
            (Some("Which branch?".into()), Some(30))
        );
        assert!(parse_listen(&["--timeout", "soon"]).is_err());
        assert!(parse_listen(&["--prompt"]).is_err());
        assert!(parse_listen(&["--loud"]).is_err());
    }

    #[test]
    fn mcp_lists_tools_and_answers_initialize() {
        let init = mcp_handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "mouthpiece");

        let list = mcp_handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["ask_user", "add_vocabulary", "mouthpiece_status"]);
    }

    #[test]
    fn mcp_ignores_notifications_and_rejects_unknown_methods() {
        assert!(
            mcp_handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none()
        );
        let err = mcp_handle(&json!({"jsonrpc":"2.0","id":3,"method":"resources/list"})).unwrap();
        assert_eq!(err["error"]["code"], -32601);
    }

    #[test]
    fn mcp_serve_reads_lines_and_reports_bad_json() {
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\nnot json\n\n";
        let mut out = Vec::new();
        mcp_serve(&input[..], &mut out);
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["result"], json!({}));
        assert_eq!(lines[1]["error"]["code"], -32700);
    }

    #[test]
    fn requests_round_trip_over_the_socket() {
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("mp-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            for reply in [
                r#"{"ok":true,"text":"use main"}"#,
                r#"{"ok":false,"error":"busy"}"#,
            ] {
                let (stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                assert!(line.contains("\"cmd\""));
                writeln!(&stream, "{reply}").unwrap();
            }
        });
        let ok = request_at(&path, &json!({"cmd": "listen"}), Duration::from_secs(5)).unwrap();
        assert_eq!(ok["text"], "use main");
        let err = request_at(&path, &json!({"cmd": "listen"}), Duration::from_secs(5)).unwrap_err();
        assert_eq!(err, "busy");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(request_at(&path, &json!({}), Duration::from_secs(1))
            .unwrap_err()
            .contains("isn't running"));
    }

    #[test]
    fn wrapper_quotes_the_path() {
        let script = wrapper_script(std::path::Path::new(
            "/Applications/It's Mouthpiece.app/Contents/MacOS/tauri-app",
        ));
        assert!(script.contains(
            r"exec '/Applications/It'\''s Mouthpiece.app/Contents/MacOS/tauri-app' --mouthpiece-cli"
        ));
    }
}
