#[allow(dead_code)]
#[path = "../src/device_login.rs"]
mod device_login;
#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/pane_header.rs"]
mod pane_header;
#[allow(dead_code)]
#[path = "../src/pane_menu_model.rs"]
mod pane_menu_model;
#[allow(dead_code)]
#[path = "../src/rich_previews.rs"]
mod rich_previews;
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/tessel_sync.rs"]
mod tessel_sync;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/vault.rs"]
mod vault;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;
use serde_json::json;

#[cfg(windows)]
#[test]
fn encrypted_sync_crypto_dpapi_floor_and_http_roundtrip() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use tessel_sync::*;

    // 1. NFKC normalization + phrase generator + weakness checks
    assert_eq!(
        normalize_sync_phrase("  ﬁnal-phrase-1234  "),
        "final-phrase-1234"
    );
    let generated = suggest_phrase().expect("phrase generated");
    assert_eq!(generated.len(), 29);
    assert!(phrase_is_weak(&generated).is_none());
    assert!(phrase_is_weak("password1234").is_some());

    // 2. Key derivation + DPAPI local cache + shrink guard + anti-rollback floor
    let dir = tempfile::tempdir().unwrap();
    let salt = [0x42_u8; 16];
    let key = scrypt_derive_key(&generated, &salt, 12, SCRYPT_R, SCRYPT_P).unwrap();
    save_dpapi_sync_key(dir.path(), "user-abc", &salt, &key).unwrap();
    let (loaded_salt, loaded_key) = load_dpapi_sync_key(dir.path(), "user-abc")
        .unwrap()
        .expect("cached key");
    assert_eq!(loaded_salt, salt);
    assert_eq!(&*loaded_key, &*key);
    assert!(
        load_dpapi_sync_key(dir.path(), "other-user")
            .unwrap()
            .is_none()
    );

    assert!(check_shrink_guard(6, 2).is_some());
    assert!(check_shrink_guard(6, 4).is_none());

    let raw_ws = json!({
        "schema": 1,
        "projects": [{"id": 1, "name": "Alpha"}],
        "native_threads": {"1": {"project": 1, "provider": "Claude", "thread": "secret"}},
        "provider_accounts_prefs": {"claude": "strip"},
        "sessions": [{"id": 1, "conversation_id": "strip-id", "title": "Claude"}]
    });
    let sanitized = sanitize_workspace_json_for_sync(raw_ws);
    assert!(sanitized.get("native_threads").is_none());
    assert!(sanitized.get("provider_accounts_prefs").is_none());
    assert!(sanitized["sessions"][0].get("conversation_id").is_none());

    let envelope = build_rust_sync_envelope(1, "Test-PC", 1_710_000_000_000, sanitized);
    let ciphertext = encrypt_sync_envelope(&key, "user-abc", &envelope).unwrap();
    let salt_b64 = URL_SAFE_NO_PAD.encode(salt);

    // 3. Local HTTP mock server verifying GET meta, PUT upload, GET snapshot, GET versions, DELETE
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    type StoredSnapshot = Option<(u64, String, String)>;
    let stored: Arc<Mutex<StoredSnapshot>> = Arc::new(Mutex::new(None));
    let server_store = Arc::clone(&stored);

    let handle = thread::spawn(move || {
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 262_144];
            let mut total = 0usize;
            loop {
                let n = stream.read(&mut buf[total..]).unwrap_or(0);
                if n == 0 {
                    break;
                }
                total += n;
                if let Some(hdr_end) = buf[..total].windows(4).position(|w| w == b"\r\n\r\n") {
                    let hdr = String::from_utf8_lossy(&buf[..hdr_end]);
                    let content_len = hdr
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if total >= hdr_end + 4 + content_len {
                        break;
                    }
                }
            }
            let req_str = String::from_utf8_lossy(&buf[..total]);
            let first_line = req_str.lines().next().unwrap_or("");
            let body_start = req_str.find("\r\n\r\n").map_or(total, |i| i + 4);
            let body_str = &req_str[body_start..];

            let (status, resp_json) = if first_line.starts_with("GET /api/sync?meta=1") {
                let guard = server_store.lock().unwrap();
                match &*guard {
                    None => ("404 Not Found", json!({"error": "none"})),
                    Some((seq, s, ct)) => {
                        ("200 OK", json!({"seq": seq, "salt": s, "size": ct.len()}))
                    }
                }
            } else if first_line.starts_with("PUT /api/sync") {
                let parsed: serde_json::Value = serde_json::from_str(body_str).unwrap();
                let next_seq = parsed["expected_seq"].as_u64().unwrap_or(0) + 1;
                let s = parsed["salt"].as_str().unwrap().to_owned();
                let ct = parsed["ciphertext"].as_str().unwrap().to_owned();
                *server_store.lock().unwrap() = Some((next_seq, s, ct));
                ("200 OK", json!({"seq": next_seq}))
            } else if first_line.starts_with("GET /api/sync/versions") {
                let guard = server_store.lock().unwrap();
                let versions = guard
                    .as_ref()
                    .map(|(seq, _, ct)| {
                        vec![json!({"seq": seq, "updated_at": 1000, "size": ct.len()})]
                    })
                    .unwrap_or_default();
                ("200 OK", json!({"versions": versions}))
            } else if first_line.starts_with("GET /api/sync ") {
                let guard = server_store.lock().unwrap();
                match &*guard {
                    None => ("404 Not Found", json!({"error": "none"})),
                    Some((seq, s, ct)) => {
                        ("200 OK", json!({"seq": seq, "salt": s, "ciphertext": ct}))
                    }
                }
            } else if first_line.starts_with("DELETE /api/sync") {
                *server_store.lock().unwrap() = None;
                ("200 OK", json!({"deleted": true}))
            } else {
                ("400 Bad Request", json!({"error": "unexpected"}))
            };
            let payload = serde_json::to_vec(&resp_json).unwrap();
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&payload);
        }
    });

    let client = SyncHttpClient::with_base(&format!("http://{addr}")).unwrap();
    assert!(client.fetch_meta("tok").unwrap().is_none());
    let uploaded_seq = client
        .upload_snapshot("tok", 0, &salt_b64, &ciphertext)
        .unwrap();
    assert_eq!(uploaded_seq, 1);

    let versions = client.list_versions("tok").unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].seq, 1);

    let (meta, fetched_ct) = client.fetch_snapshot("tok").unwrap().unwrap();
    assert_eq!(meta.seq, 1);
    let dec = decrypt_sync_envelope(&key, "user-abc", &fetched_ct).unwrap();
    let mut floor = SyncFloorFile::load(dir.path());
    let (seq, bundle) = extract_rust_bundle_from_envelope(&dec, 0, false).unwrap();
    floor.record_seq(dir.path(), "user-abc", seq).unwrap();
    assert_eq!(bundle["projects"][0]["name"], "Alpha");

    client.delete_cloud_sync("tok").unwrap();
    handle.join().unwrap();
}

#[test]
fn rich_previews_sheet_json_mermaid_and_headless_ui_render() {
    use rich_previews::*;

    let sheet = parse_delimited_sheet("City;Pop\nMontréal;1780000\nQuébec;549000\n", None);
    assert_eq!(sheet.delimiter_label, "CSV (;)");
    assert_eq!(sheet.rows.len(), 3);
    assert_eq!(sheet.rows[1][0], "Montréal");

    let jsonc = "// header\n{\"enabled\": true, \"ports\": [8080, 9090,],}\n";
    let parsed_json = parse_json_or_jsonl(jsonc, "jsonc").unwrap();
    assert_eq!(parsed_json.root["enabled"], true);

    let mermaid_flow = "flowchart TD\n  Start([User]) --> Auth{Signed in?}\n  Auth -->|Yes| Sync[Encrypted Sync]\n";
    assert!(parse_mermaid_diagram(mermaid_flow).is_ok());

    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            show_sheet_grid(ui, "test-sheet", &sheet);
            show_json_preview(ui, "test-json", jsonc, "jsonc");
            show_mermaid_block(ui, mermaid_flow);
            show_mermaid_block(ui, "pie title Unsupported\n  \"A\" : 40");
        });
    });
}

#[test]
fn device_login_banner_and_chat_switch_menu_items_work_headless() {
    use device_login::*;
    use pane_header::MenuState;
    use pane_menu_model::{
        PaneActionsMenuInput, PaneMenuAction, PaneMenuEntry, PaneMenuLang, build_pane_actions_menu,
    };

    let screen = "Please open https://console.anthropic.com/oauth/authorize?client_id=abc123\nand enter code WXYZ-4321 to continue.";
    let prompt = detect_device_login_prompt(screen, 100).expect("OAuth prompt detected");
    assert!(prompt.known_host);
    assert_eq!(prompt.device_code.as_deref(), Some("WXYZ-4321"));

    let built_term = build_pane_actions_menu(
        &PaneActionsMenuInput {
            is_agent: true,
            can_open_as_chat: true,
            chat_view_available: true,
            chat_view_open: false,
            ..Default::default()
        },
        PaneMenuLang::En,
    );
    assert!(built_term.entries.iter().any(|e| matches!(
        e,
        PaneMenuEntry::Item {
            action: PaneMenuAction::OpenOrSwitchChat,
            enabled: true,
            ..
        }
    )));

    let built_chat = build_pane_actions_menu(
        &PaneActionsMenuInput {
            is_agent: true,
            can_open_as_chat: true,
            chat_view_available: true,
            chat_view_open: true,
            ..Default::default()
        },
        PaneMenuLang::En,
    );
    assert!(built_chat.entries.iter().any(|e| matches!(
        e,
        PaneMenuEntry::Item {
            action: PaneMenuAction::OpenOrSwitchChat,
            enabled: true,
            ..
        }
    )));

    let term_menu_state = MenuState {
        can_switch_chat_mode: true,
        chat_view_open: false,
        is_agent: true,
        language: PaneMenuLang::En,
        ..Default::default()
    };

    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = pane_header::show(ui, &term_menu_state);
            let _ = show_device_login_banner(ui, &prompt, false);
            let _ = show_device_login_banner(ui, &prompt, true);
        });
    });
}
