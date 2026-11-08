//! Integration tests for Lot b179:
//! 1. Hardened built-in Browser OAuth sign-in popups (`a93791f3`)
//! 2. Terminal Conversations `promptBefore` backwards transcript scan (`c4686b4e`)

#[path = "../src/browser_popup.rs"]
mod browser_popup;

#[path = "../src/prompt_before.rs"]
mod prompt_before;

use browser_popup::{
    POPUP_DEFAULT_HEIGHT, POPUP_DEFAULT_WIDTH, POPUP_GESTURE_MS, POPUP_MAX_HEIGHT, POPUP_MAX_WIDTH,
    POPUP_MIN_SIZE, ParentRect, PopupGestureTracker, has_popup_size_feature, is_popup_url_allowed,
    popup_window_rect, should_open_oauth_popup,
};
use prompt_before::{
    PromptBeforeCache, REMOTE_MAX_PROMPT_LINE_BYTES, REMOTE_TPROMPT_STDOUT_CAP, ScanOptions,
    build_remote_tprompt_awk_command, is_peer_prompt_line, is_prompt_line, last_prompt_before,
    last_prompt_before_with_options, parse_remote_tprompt_output, read_claude_window_messages,
    validate_remote_tprompt_args,
};
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn lot_b179_oauth_popup_size_feature_detection_and_geometry_clamping() {
    assert!(has_popup_size_feature("width=500,height=650"));
    assert!(has_popup_size_feature("popup=1, innerWidth = 480"));
    assert!(has_popup_size_feature("resizable=yes;INNERHEIGHT=720"));
    assert!(!has_popup_size_feature(""));
    assert!(!has_popup_size_feature("noopener,noreferrer,popup=1"));
    assert!(!has_popup_size_feature("maxwidth=500,minheight=400"));

    let parent = ParentRect {
        x: 100,
        y: 50,
        width: 1200,
        height: 900,
    };

    // Clamps below minimum (300) and above maximum (1400 x 1000)
    let small = popup_window_rect(Some((120.0, 80.0)), None, Some(parent));
    assert_eq!(small.width, POPUP_MIN_SIZE);
    assert_eq!(small.height, POPUP_MIN_SIZE);
    assert_eq!(small.x, 100 + (1200 - 300) / 2);
    assert_eq!(small.y, 50 + (900 - 300) / 2);

    let huge = popup_window_rect(Some((2400.0, 1900.0)), None, Some(parent));
    assert_eq!(huge.width, POPUP_MAX_WIDTH);
    assert_eq!(huge.height, POPUP_MAX_HEIGHT);
    assert_eq!(huge.x, 100 + (1200 - 1400) / 2);
    assert_eq!(huge.y, 50 + (900 - 1000) / 2);

    // Partial dimension falls back to 600 x 700 default for the omitted axis
    let width_only = popup_window_rect(None, Some("width=480"), Some(parent));
    assert_eq!(width_only.width, 480);
    assert_eq!(width_only.height, POPUP_DEFAULT_HEIGHT);

    let defaults = popup_window_rect(None, Some(""), Some(parent));
    assert_eq!(defaults.width, POPUP_DEFAULT_WIDTH);
    assert_eq!(defaults.height, POPUP_DEFAULT_HEIGHT);
}

#[test]
fn lot_b179_oauth_popup_gating_url_safety_and_single_use_gesture() {
    assert!(is_popup_url_allowed(
        "https://accounts.google.com/o/oauth2/v2/auth"
    ));
    assert!(is_popup_url_allowed("http://localhost:3000/auth/callback"));
    assert!(!is_popup_url_allowed("about:blank"));
    assert!(!is_popup_url_allowed("javascript:alert(1)"));
    assert!(!is_popup_url_allowed("file:///C:/Windows/win.ini"));
    assert!(!is_popup_url_allowed("data:text/html,<h1>hi</h1>"));
    assert!(!is_popup_url_allowed("myapp://oauth/callback"));
    assert!(!is_popup_url_allowed(
        "https://user:pass@accounts.example.com/login"
    ));

    let url = "https://github.com/login/oauth/authorize";
    assert!(should_open_oauth_popup(url, true, true, false, false));
    // Unsized _blank stays in-pane
    assert!(!should_open_oauth_popup(url, true, false, false, false));
    // Non-gesture blocked
    assert!(!should_open_oauth_popup(url, false, true, false, false));
    // Agent-controlled or agent-session browser blocked
    assert!(!should_open_oauth_popup(url, true, true, true, false));
    assert!(!should_open_oauth_popup(url, true, true, false, true));

    // Gesture tracker: <= 1500 ms, single popup per click, blocks double-open burst
    let mut tracker = PopupGestureTracker::new();
    assert!(!tracker.consume_if_fresh(1_000));
    tracker.record_gesture(10_000);
    assert!(tracker.consume_if_fresh(10_000 + POPUP_GESTURE_MS));
    // Consumed: second popup on same click is denied
    assert!(!tracker.consume_if_fresh(10_000 + POPUP_GESTURE_MS));
    // Synchronous re-arm within < 250 ms burst is ignored
    tracker.record_gesture(10_000 + POPUP_GESTURE_MS + 50);
    assert!(!tracker.consume_if_fresh(10_000 + POPUP_GESTURE_MS + 60));
    // Expired gesture (> 1500 ms) is rejected
    tracker.record_gesture(20_000);
    assert!(!tracker.consume_if_fresh(20_000 + POPUP_GESTURE_MS + 1));
}

#[test]
fn lot_b179_is_prompt_line_classifies_human_queued_peer_tool_meta_and_sidechain() {
    let human = r#"{"type":"user","message":{"role":"user","content":"Refactor promptBefore"}}"#;
    let human_origin =
        r#"{"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"Hi"}}"#;
    let queued_prompt = r#"{"type":"queue-operation","queued_command":{"commandMode":"prompt","prompt":"Next step"}}"#;
    let peer_meta = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","agent":"Ada"},"message":{"role":"user","content":"Ping"}}"#;
    let tool_res = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#;
    let meta_non_peer =
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command>"}}"#;
    let task_notice = r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"role":"user","content":"done"}}"#;
    let sidechain = r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"subagent prompt"}}"#;
    let queued_bash =
        r#"{"type":"queue-operation","queued_command":{"commandMode":"bash","prompt":"ls"}}"#;

    assert!(is_prompt_line(human));
    assert!(!is_peer_prompt_line(human));
    assert!(is_prompt_line(human_origin));
    assert!(is_prompt_line(queued_prompt));
    assert!(is_prompt_line(peer_meta));
    assert!(is_peer_prompt_line(peer_meta));

    assert!(!is_prompt_line(tool_res));
    assert!(!is_prompt_line(meta_non_peer));
    assert!(!is_prompt_line(task_notice));
    assert!(!is_prompt_line(sidechain));
    assert!(!is_prompt_line(queued_bash));
}

#[test]
fn lot_b179_last_prompt_before_scans_across_megabytes_cut_lines_stop_at_and_budget() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("session.jsonl");

    let prompt_1 = r#"{"type":"user","message":{"role":"user","content":"First user prompt"}}"#;
    let prompt_2 = r#"{"type":"user","message":{"role":"user","content":"Second user prompt before big tool chain"}}"#;
    let big_tool_payload = "x".repeat(600 * 1024);
    let tool_line = format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":"{big_tool_payload}"}}]}}}}"#
    );
    let assistant_end = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Final answer after 2.4 MiB of tool outputs"}]}}"#;

    let mut content = String::new();
    let p1_offset = 0_u64;
    content.push_str(prompt_1);
    content.push('\n');
    let p2_offset = content.len() as u64;
    content.push_str(prompt_2);
    content.push('\n');
    for _ in 0..4 {
        content.push_str(&tool_line);
        content.push('\n');
    }
    let tail_offset = content.len() as u64;
    content.push_str(assistant_end);
    content.push('\n');
    fs::write(&path, content.as_bytes()).expect("write transcript");
    let _ = p1_offset;

    // 1. Default multi-chunk scan finds `prompt_2` across > 2.4 MiB of tool_result lines
    let found = last_prompt_before(&path, tail_offset)
        .expect("io ok")
        .expect("prompt_2 found");
    assert_eq!(found, prompt_2);

    // 2. Small 128 KiB chunk size still reconstructs lines across chunk boundaries
    let found_small_chunks = last_prompt_before_with_options(
        &path,
        tail_offset,
        ScanOptions {
            chunk: 128 * 1024,
            ..ScanOptions::default()
        },
    )
    .expect("io ok")
    .expect("prompt_2 found with 128 KiB chunks");
    assert_eq!(found_small_chunks, prompt_2);

    // 3. `before` cutting right through the middle of `prompt_2` reads forward carry and recovers `prompt_2`
    let mid_p2 = p2_offset + 18;
    let found_cut = last_prompt_before(&path, mid_p2)
        .expect("io ok")
        .expect("cut prompt_2 recovered");
    assert_eq!(found_cut, prompt_2);

    // 4. `stop_at` landing mid-line inside `prompt_2` advances to next newline and stops cleanly
    let stopped = last_prompt_before_with_options(
        &path,
        tail_offset,
        ScanOptions {
            stop_at: p2_offset + 5,
            ..ScanOptions::default()
        },
    )
    .expect("io ok");
    assert!(stopped.is_none());

    // 5. Tight budget smaller than the tool_result gap returns None without runaway scanning
    let budget_capped = last_prompt_before_with_options(
        &path,
        tail_offset,
        ScanOptions {
            chunk: 64 * 1024,
            budget: 256 * 1024,
            ..ScanOptions::default()
        },
    )
    .expect("io ok");
    assert!(budget_capped.is_none());
}

#[test]
fn lot_b179_prompt_before_cache_incremental_window_and_error_retry_and_window_reader() {
    let mut cache = PromptBeforeCache::new();
    let mut calls = Vec::new();

    // Initial lookup at head = 1000 scans from since = 0
    let r1: Result<Option<String>, &str> = cache.resolve_with(1_000, |before, since| {
        calls.push((before, since));
        Ok(Some("prompt-A".to_owned()))
    });
    assert_eq!(r1.unwrap().as_deref(), Some("prompt-A"));
    assert_eq!(calls, vec![(1_000, 0)]);

    // Identical head = 1000 uses cache without invoking finder
    let r2: Result<Option<String>, &str> = cache.resolve_with(1_000, |_, _| {
        panic!("finder must not run when head == prompt_at");
    });
    assert_eq!(r2.unwrap().as_deref(), Some("prompt-A"));

    // Transient error at head = 2000 preserves previous (prompt_at=1000, prompt_line="prompt-A")
    let err_res: Result<Option<String>, &str> =
        cache.resolve_with(2_000, |_, _| Err("ssh timeout"));
    assert_eq!(err_res, Err("ssh timeout"));
    assert_eq!(cache.prompt_at(), Some(1_000));
    assert_eq!(cache.prompt_line(), Some("prompt-A"));

    // Next poll at head = 2000 retries incrementally [1000..2000] and retains "prompt-A" when None in delta
    let r3: Result<Option<String>, &str> = cache.resolve_with(2_000, |before, since| {
        calls.push((before, since));
        Ok(None)
    });
    assert_eq!(r3.unwrap().as_deref(), Some("prompt-A"));
    assert_eq!(calls, vec![(1_000, 0), (2_000, 1_000)]);

    // Window reader recovers human turn across big tool output AND hides peer turn reply
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("conv.jsonl");
    let big = "z".repeat(40 * 1024);
    let jsonl = [
        r#"{"type":"user","message":{"role":"user","content":"Explain the architecture"}}"#
            .to_owned(),
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":"{big}"}}]}}}}"#
        ),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Here is the architecture summary."}]}}"#
            .to_owned(),
        r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","agent":"Bob"},"message":{"role":"user","content":"Peer ping"}}"#
            .to_owned(),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hidden peer reply"}]}}"#
            .to_owned(),
    ]
    .join("\n")
        + "\n";
    fs::write(&path, jsonl).expect("write conv.jsonl");

    let mut win_cache = PromptBeforeCache::new();
    let turns = read_claude_window_messages(&path, 8 * 1024, &mut win_cache, 1_700_000_000_000)
        .expect("read window");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].sender, "You");
    assert_eq!(turns[0].body, "Explain the architecture");
    assert_eq!(turns[1].sender, "Claude");
    assert_eq!(turns[1].body, "Here is the architecture summary.");
}

#[test]
fn lot_b179_remote_tprompt_awk_validation_parser_and_posix_execution() {
    let valid_id = "12345678-1234-4234-8234-123456789abc";
    assert!(validate_remote_tprompt_args("claude", valid_id, 500, 0).is_ok());
    assert!(validate_remote_tprompt_args("codex", valid_id, 500, 0).is_err());
    assert!(validate_remote_tprompt_args("claude", "../escape", 500, 0).is_err());
    assert!(validate_remote_tprompt_args("claude", "short", 500, 0).is_err());
    assert!(validate_remote_tprompt_args("claude", valid_id, 0, 0).is_err());
    assert!(validate_remote_tprompt_args("claude", valid_id, 100, 200).is_err());

    assert_eq!(parse_remote_tprompt_output("").unwrap(), None);
    assert_eq!(parse_remote_tprompt_output("  \n ").unwrap(), None);
    assert_eq!(
        parse_remote_tprompt_output("1024 88\n").unwrap(),
        Some((1024, 88))
    );
    assert!(parse_remote_tprompt_output("1024 0\n").is_err());
    assert!(
        parse_remote_tprompt_output(&format!("0 {}\n", REMOTE_MAX_PROMPT_LINE_BYTES + 1)).is_err()
    );
    assert!(parse_remote_tprompt_output(&"9".repeat(REMOTE_TPROMPT_STDOUT_CAP + 1)).is_err());

    // Execute the generated POSIX `LC_ALL=C awk` script via Git's `sh.exe` when available
    let git_sh = Path::new(r"C:\Program Files\Git\bin\sh.exe");
    if git_sh.is_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let proj = tmp
            .path()
            .join(".claude")
            .join("projects")
            .join("demo-proj");
        fs::create_dir_all(&proj).expect("mkdir proj");
        let file_path = proj.join(format!("{valid_id}.jsonl"));
        let l1 = r#"{"type":"user","message":{"role":"user","content":"Remote prompt one"}}"#;
        let l2 = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ignored"}]}}"#;
        let l3 = r#"{"type":"assistant","message":{"role":"assistant","content":"Answer"}}"#;
        let body = format!("{l1}\n{l2}\n{l3}\n");
        fs::write(&file_path, body.as_bytes()).expect("write remote transcript");

        let before = (l1.len() + 1 + l2.len() + 1) as u64;
        let script = build_remote_tprompt_awk_command("claude", valid_id, before, 0)
            .expect("build awk script");
        let home_posix = tmp.path().to_string_lossy().replace('\\', "/");
        let output = Command::new(git_sh)
            .arg("-c")
            .arg(&script)
            .env("HOME", home_posix)
            .output()
            .expect("run sh -c awk");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = parse_remote_tprompt_output(&stdout)
            .expect("parse awk output")
            .expect("matched offset & len");
        assert_eq!(parsed, (0, l1.len()));
    }
}
