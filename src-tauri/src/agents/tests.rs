use super::*;

fn url(s: &str) -> tauri::Url {
    tauri::Url::parse(s).unwrap()
}

fn report(pid: Option<i32>) -> Report {
    Report {
        session_id: "s1".into(),
        cwd: "/work/dev-whisper".into(),
        message: String::new(),
        kind: Kind::NeedsInput,
        pid,
        bundle: None,
        term_program: None,
    }
}

fn agent(session: &str, received_ms: u64) -> WaitingAgent {
    WaitingAgent {
        session_id: session.into(),
        cwd: "/work/app".into(),
        folder: "app".into(),
        message: "needs permission".into(),
        kind: Kind::NeedsInput,
        app_pid: Some(10),
        bundle_id: None,
        app_name: None,
        tty: None,
        received_ms,
    }
}

// ---- deep link parsing

#[test]
fn parses_a_waiting_link_with_percent_encoding() {
    let link = url("mouthpiece://agent-waiting?session=abc&cwd=%2FUsers%2Fme%2Fmy%20app&message=Claude%20needs%20permission&kind=needs_input&pid=4242&bundle=com.apple.Terminal&term=Apple_Terminal");
    let LinkEvent::Waiting(r) = parse_link(&link).unwrap() else {
        panic!("expected a waiting event");
    };
    assert_eq!(r.session_id, "abc");
    assert_eq!(r.cwd, "/Users/me/my app");
    assert_eq!(r.message, "Claude needs permission");
    assert_eq!(r.kind, Kind::NeedsInput);
    assert_eq!(r.pid, Some(4242));
    assert_eq!(r.bundle.as_deref(), Some("com.apple.Terminal"));
    assert_eq!(r.term_program.as_deref(), Some("Apple_Terminal"));
}

#[test]
fn done_kind_and_missing_optionals() {
    let LinkEvent::Waiting(r) =
        parse_link(&url("mouthpiece://agent-waiting?session=a&kind=done")).unwrap()
    else {
        panic!()
    };
    assert_eq!(r.kind, Kind::Done);
    assert_eq!((r.pid, r.bundle, r.cwd.as_str()), (None, None, ""));
}

#[test]
fn a_link_without_session_falls_back_to_the_folder() {
    let LinkEvent::Waiting(r) =
        parse_link(&url("mouthpiece://agent-waiting?cwd=%2Fwork%2Fapp")).unwrap()
    else {
        panic!()
    };
    assert_eq!(r.session_id, "cwd:/work/app");
}

#[test]
fn rejects_links_with_nothing_to_identify_the_agent() {
    assert!(parse_link(&url("mouthpiece://agent-waiting?message=hi")).is_err());
    assert!(parse_link(&url("mouthpiece://agent-waiting")).is_err());
    assert!(parse_link(&url("mouthpiece://agent-bogus?session=a")).is_err());
}

#[test]
fn pid_one_and_garbage_are_ignored() {
    for pid in ["1", "0", "-5", "abc"] {
        let LinkEvent::Waiting(r) = parse_link(&url(&format!(
            "mouthpiece://agent-waiting?session=a&pid={pid}"
        )))
        .unwrap() else {
            panic!()
        };
        assert_eq!(r.pid, None, "pid={pid}");
    }
}

#[test]
fn resumed_link_names_the_session() {
    assert_eq!(
        parse_link(&url("mouthpiece://agent-resumed?session=abc&cwd=%2Fx")).unwrap(),
        LinkEvent::Resumed {
            session_id: "abc".into()
        }
    );
}

#[test]
fn long_multiline_messages_are_flattened_and_trimmed() {
    let long = "word ".repeat(100);
    let msg = clean_message(&format!("first\nsecond\t{long}"));
    assert!(!msg.contains('\n') && !msg.contains('\t'));
    assert!(msg.starts_with("first second word"));
    assert_eq!(msg.chars().count(), MAX_MESSAGE_CHARS);
    assert!(msg.ends_with('…'));
}

#[test]
fn folder_is_the_last_path_component() {
    assert_eq!(folder_name("/Users/me/dev-whisper"), "dev-whisper");
    assert_eq!(folder_name("/Users/me/dev-whisper/"), "dev-whisper");
    assert_eq!(folder_name("/"), "agent");
    assert_eq!(folder_name(""), "agent");
}

// ---- queue

#[test]
fn newest_agent_is_first_and_sessions_dedupe() {
    let mut q = AgentQueue::default();
    assert!(q.upsert(agent("a", 1_000), 1_000));
    assert!(q.upsert(agent("b", 2_000), 2_000));
    assert_eq!(q.latest(2_000).unwrap().session_id, "b");

    // "a" asks again with new words: still one entry for it, now the newest.
    let mut again = agent("a", 3_000);
    again.message = "something else".into();
    assert!(q.upsert(again, 3_000));
    let ids: Vec<_> = q.list(3_000).into_iter().map(|a| a.session_id).collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn an_identical_repeat_does_not_notify_again() {
    let mut q = AgentQueue::default();
    assert!(q.upsert(agent("a", 1_000), 1_000));
    assert!(!q.upsert(agent("a", 2_000), 2_000));
    assert_eq!(q.list(2_000).len(), 1);
    assert_eq!(q.latest(2_000).unwrap().received_ms, 2_000);
}

#[test]
fn entries_expire_after_thirty_minutes() {
    let mut q = AgentQueue::default();
    q.upsert(agent("old", 0), 0);
    q.upsert(agent("new", EXPIRY_MS - 1), EXPIRY_MS - 1);
    assert_eq!(q.list(EXPIRY_MS - 1).len(), 2);
    let ids: Vec<_> = q
        .list(EXPIRY_MS)
        .into_iter()
        .map(|a| a.session_id)
        .collect();
    assert_eq!(ids, ["new"]);
    assert!(q.latest(EXPIRY_MS * 2).is_none());
    assert!(q.get("new", EXPIRY_MS * 2).is_none());
}

#[test]
fn resuming_removes_the_session() {
    let mut q = AgentQueue::default();
    q.upsert(agent("a", 0), 0);
    assert!(q.remove("a"));
    assert!(!q.remove("a"));
    assert!(q.latest(0).is_none());
}

#[test]
fn the_queue_is_capped() {
    let mut q = AgentQueue::default();
    for i in 0..20u64 {
        q.upsert(agent(&format!("s{i}"), i), i);
    }
    assert_eq!(q.list(20).len(), MAX_WAITING);
    assert_eq!(q.latest(20).unwrap().session_id, "s19");
}

// ---- owner resolution with faked process data

fn tree(pid: i32) -> Option<ProcInfo> {
    // 500 (claude) -> 400 (zsh, on ttys004) -> 300 (login) -> 200 (Terminal app)
    match pid {
        500 => Some(ProcInfo {
            ppid: 400,
            tty: None,
        }),
        400 => Some(ProcInfo {
            ppid: 300,
            tty: Some("ttys004".into()),
        }),
        300 => Some(ProcInfo {
            ppid: 200,
            tty: Some("ttys004".into()),
        }),
        _ => None,
    }
}

fn app_pid(pid: i32) -> Option<(String, String)> {
    (pid == 200).then(|| ("com.apple.Terminal".to_string(), "Terminal".to_string()))
}

fn no_bundle(_: &str) -> Option<(i32, String)> {
    None
}

#[test]
fn walks_up_to_the_owning_app_and_remembers_the_tty() {
    let owner = resolve_owner(&report(Some(500)), &tree, &app_pid, &no_bundle).unwrap();
    assert_eq!(
        owner,
        Owner {
            pid: 200,
            bundle_id: "com.apple.Terminal".into(),
            name: "Terminal".into(),
            tty: Some("ttys004".into())
        }
    );
}

#[test]
fn a_dead_end_falls_back_to_the_exported_bundle_id() {
    let mut r = report(Some(999));
    r.bundle = Some("com.googlecode.iterm2".into());
    let by_bundle = |b: &str| (b == "com.googlecode.iterm2").then(|| (77, "iTerm".to_string()));
    let owner = resolve_owner(&r, &tree, &app_pid, &by_bundle).unwrap();
    assert_eq!((owner.pid, owner.name.as_str()), (77, "iTerm"));
}

#[test]
fn term_program_is_the_last_resort() {
    let mut r = report(None);
    r.term_program = Some("Apple_Terminal".into());
    let by_bundle = |b: &str| (b == "com.apple.Terminal").then(|| (200, "Terminal".to_string()));
    assert_eq!(
        resolve_owner(&r, &tree, &app_pid, &by_bundle)
            .unwrap()
            .bundle_id,
        "com.apple.Terminal"
    );
    r.term_program = Some("SomethingElse".into());
    assert!(resolve_owner(&r, &tree, &app_pid, &by_bundle).is_none());
}

#[test]
fn a_process_loop_or_launchd_stops_the_walk() {
    // 2 -> 3 -> 2 -> ...
    let looping = |pid: i32| {
        Some(ProcInfo {
            ppid: if pid == 2 { 3 } else { 2 },
            tty: None,
        })
    };
    assert!(resolve_owner(&report(Some(2)), &looping, &|_| None, &no_bundle).is_none());
    let to_launchd = |_: i32| Some(ProcInfo { ppid: 1, tty: None });
    assert!(resolve_owner(&report(Some(50)), &to_launchd, &|_| None, &no_bundle).is_none());
}

#[test]
fn parses_ps_output() {
    let info = parse_ps("  412 ttys003\n").unwrap();
    assert_eq!((info.ppid, info.tty.as_deref()), (412, Some("ttys003")));
    let info = parse_ps("  1 ??\n").unwrap();
    assert_eq!(info.tty, None);
    assert!(parse_ps("").is_none());
}

// ---- window choice

fn titles(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

#[test]
fn picks_the_window_naming_the_folder() {
    let t = titles(&["notes — zsh", "dev-whisper — claude — 80×24", "other"]);
    assert_eq!(
        choose_window(&t, "dev-whisper", None),
        Ok(WindowChoice::Match(1))
    );
    assert_eq!(
        choose_window(&t, "DEV-WHISPER", None),
        Ok(WindowChoice::Match(1))
    );
}

#[test]
fn the_tty_wins_over_the_folder() {
    let t = titles(&["dev-whisper — a", "dev-whisper — b — ttys004"]);
    assert_eq!(
        choose_window(&t, "dev-whisper", Some("ttys004")),
        Ok(WindowChoice::Tty(1))
    );
}

#[test]
fn refuses_to_guess_between_windows() {
    let ambiguous = titles(&["dev-whisper — a", "dev-whisper — b"]);
    assert!(choose_window(&ambiguous, "dev-whisper", None).is_err());
    let unrelated = titles(&["one", "two"]);
    assert!(choose_window(&unrelated, "dev-whisper", None).is_err());
    assert!(choose_window(&[], "dev-whisper", None).is_err());
}

#[test]
fn a_lone_window_is_used_even_if_its_title_does_not_say() {
    assert_eq!(
        choose_window(&titles(&["Orca"]), "dev-whisper", None),
        Ok(WindowChoice::Only(0))
    );
}

// ---- reply wording

#[test]
fn send_it_becomes_press_enter() {
    assert_eq!(
        normalize_reply("Yes, go ahead. Send it."),
        "Yes, go ahead. press enter"
    );
    assert_eq!(normalize_reply("send it"), "press enter");
    assert_eq!(
        normalize_reply("Approve it, send that!"),
        "Approve it, press enter"
    );
}

#[test]
fn send_it_inside_a_sentence_is_left_alone() {
    assert_eq!(normalize_reply("Don't send it yet"), "Don't send it yet");
    assert_eq!(normalize_reply("resend it"), "resend it");
    assert_eq!(
        normalize_reply("Fix the bug and send it"),
        "Fix the bug and send it"
    );
}

#[test]
fn a_reply_ending_in_send_it_gets_enter_through_the_existing_extraction() {
    let (text, enter) =
        crate::punctuation::extract_press_enter(&normalize_reply("Yes, run the tests. Send it."));
    assert!(enter);
    assert_eq!(text, "Yes, run the tests.");
}

// ---- hook config

#[test]
fn hook_config_quotes_a_path_with_spaces() {
    let path = "/Users/me/Library/Application Support/dev.mouthpiece/bin/mouthpiece-hook";
    let v: serde_json::Value = serde_json::from_str(&hook_config_json(path)).unwrap();
    for event in ["Notification", "Stop", "UserPromptSubmit"] {
        let hook = &v["hooks"][event][0]["hooks"][0];
        assert_eq!(hook["command"], format!("'{path}'"));
        assert_eq!(hook["type"], "command");
    }
}

#[test]
fn default_reply_shortcut_is_valid_and_not_the_default_push_to_talk() {
    let reply = crate::shortcut::to_shortcut(&default_reply_shortcut()).unwrap();
    let main = ShortcutConfig::default();
    assert_ne!(reply, crate::shortcut::to_shortcut(&main).unwrap());
    assert_ne!(
        reply,
        crate::shortcut::to_shortcut(&crate::shortcut::skip_variant(&main)).unwrap()
    );
}

// ---- the real hook script, with a fake `open`

const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn hook_links_carry_the_install_token() {
    let json = r#"{"session_id":"s","cwd":"/tmp/x","hook_event_name":"Stop"}"#;
    let opened = run_hook("token", json, &[]).expect("opened a link");
    let link = url(opened.trim_start_matches("-g "));
    assert!(link_has_token(&link, TEST_TOKEN));
    assert!(!link_has_token(&link, "ffffffffffffffffffffffffffffffff"));
    assert!(!link_has_token(
        &url("mouthpiece://agent-waiting?session=s&cwd=/tmp/x"),
        TEST_TOKEN
    ));
    assert!(!hook_script_with_token(TEST_TOKEN).contains(TOKEN_PLACEHOLDER));
}

fn run_hook(name: &str, stdin: &str, extra_env: &[(&str, &str)]) -> Option<String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("dw-hook-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("opened.txt");
    let fake = dir.join("fake-open");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$@\" >> '{}'\n", out.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let script = dir.join("mouthpiece-hook");
    std::fs::write(&script, hook_script_with_token(TEST_TOKEN)).unwrap();
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg(script)
        .env("DEV_WHISPER_OPEN", &fake)
        .stdin(std::process::Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    assert!(
        child.wait().unwrap().success(),
        "the hook must always exit 0"
    );
    let opened = std::fs::read_to_string(&out).ok();
    let _ = std::fs::remove_dir_all(&dir);
    opened.map(|s| s.trim().to_string())
}

fn link_of(opened: &str) -> LinkEvent {
    parse_link(&url(opened.trim_start_matches("-g "))).unwrap()
}

#[test]
fn hook_script_turns_a_notification_into_a_waiting_link() {
    let json = r#"{"session_id":"sess-1","cwd":"/Users/me/my app","hook_event_name":"Notification","message":"Claude needs your permission to use Bash"}"#;
    let opened = run_hook(
        "notification",
        json,
        &[
            ("__CFBundleIdentifier", "com.apple.Terminal"),
            ("TERM_PROGRAM", "Apple_Terminal"),
        ],
    )
    .expect("the hook should have opened a link");
    assert!(
        opened.starts_with("-g mouthpiece://agent-waiting?"),
        "{opened}"
    );
    let LinkEvent::Waiting(r) = link_of(&opened) else {
        panic!("expected a waiting event");
    };
    assert_eq!(r.session_id, "sess-1");
    assert_eq!(r.cwd, "/Users/me/my app");
    assert_eq!(r.message, "Claude needs your permission to use Bash");
    assert_eq!(r.kind, Kind::NeedsInput);
    assert_eq!(r.bundle.as_deref(), Some("com.apple.Terminal"));
    assert_eq!(r.term_program.as_deref(), Some("Apple_Terminal"));
    assert!(r.pid.is_some_and(|p| p > 1), "should pass its parent pid");
}

#[test]
fn hook_script_marks_stop_events_done() {
    let json = r#"{"session_id":"sess-2","cwd":"/work/app","hook_event_name":"Stop"}"#;
    let opened = run_hook("stop", json, &[]).unwrap();
    let LinkEvent::Waiting(r) = link_of(&opened) else {
        panic!()
    };
    assert_eq!((r.session_id.as_str(), r.kind), ("sess-2", Kind::Done));
    assert_eq!(r.message, "Finished");
}

#[test]
fn hook_script_reports_the_user_answering_in_the_terminal() {
    let json = r#"{"session_id":"sess-3","cwd":"/work/app","hook_event_name":"UserPromptSubmit","prompt":"go"}"#;
    let opened = run_hook("resumed", json, &[]).unwrap();
    assert!(opened.contains("agent-resumed"), "{opened}");
    assert_eq!(
        link_of(&opened),
        LinkEvent::Resumed {
            session_id: "sess-3".into()
        }
    );
}

#[test]
fn hook_script_survives_odd_input() {
    // Quotes, unicode, an ampersand and a newline in the message must not
    // break the link, and garbage on stdin must not fail the agent.
    let json = r#"{"session_id":"s&1","cwd":"/work/café & co","hook_event_name":"Notification","message":"say \"hi\" & go\nnext"}"#;
    let opened = run_hook("odd", json, &[]).unwrap();
    let LinkEvent::Waiting(r) = link_of(&opened) else {
        panic!()
    };
    assert_eq!(r.session_id, "s&1");
    assert_eq!(r.cwd, "/work/café & co");
    assert_eq!(r.message, "say \"hi\" & go next");
    assert!(run_hook("garbage", "not json at all", &[]).is_some());
}
