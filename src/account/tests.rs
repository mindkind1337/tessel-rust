use super::*;
fn fixture_session() -> Session {
    Session {
        token: "synthetic_token_not_a_real_secret".into(),
        expires_at: now() + 60000,
        user: User {
            id: "fixture".into(),
            email: "test@example.invalid".into(),
            name: Some("Fixture".into()),
            admin: false,
        },
    }
}
#[test]
fn pkce_and_callback_validation() {
    assert_eq!(
        challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let state = "a".repeat(43);
    let uri = format!("tessel-rust://callback?state={state}&code=synthetic_code_123456");
    assert!(parse_callback(&uri).is_ok());
    for bad in [
        uri.replace("tessel-rust:", "tessel:"),
        uri.replace("callback?", "other?"),
        format!("{uri}&state={state}"),
        format!("{uri}&error=denied"),
        format!("{uri}#fragment"),
        uri.replace("callback?", "user@callback?"),
        uri.replace("callback?", "callback/path?"),
        uri.replace("callback?", "callback:80?"),
        uri.replace("synthetic_code_123456", "bad%0Acode"),
    ] {
        assert!(parse_callback(&bad).is_err());
    }
}
#[test]
fn private_storage_and_callback_are_encrypted_scoped_single_use() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    assert!(Store::open(dir.path()).is_err());
    let s = fixture_session();
    store.set(s).unwrap();
    assert!(
        store
            .update_user(User {
                id: "another-account".into(),
                email: "other@example.invalid".into(),
                name: None,
                admin: false,
            })
            .is_err()
    );
    store
        .update_user(User {
            id: "fixture".into(),
            email: "updated@example.invalid".into(),
            name: None,
            admin: false,
        })
        .unwrap();
    let bytes = fs::read(dir.path().join("session.dat")).unwrap();
    assert!(!bytes.windows(9).any(|w| w == b"synthetic"));
    let flow = store.begin().unwrap();
    let callback = format!(
        "tessel-rust://callback?state={}&code=synthetic_code_123456",
        flow.marker.state
    );
    assert!(
        accept_callback(
            dir.path(),
            &callback.replace(&flow.marker.state, &"x".repeat(43))
        )
        .is_err()
    );
    accept_callback(dir.path(), &callback).unwrap();
    assert!(accept_callback(dir.path(), &callback).is_err());
    assert_eq!(
        &**store.complete(&flow).unwrap().unwrap(),
        "synthetic_code_123456"
    );
    assert!(accept_callback(dir.path(), &callback).is_err());
    let mut expired = store.begin().unwrap();
    expired.marker.expires = now() - 1;
    save(&dir.path().join("pending.dat"), &expired.marker, false).unwrap();
    let cb = format!(
        "tessel-rust://callback?state={}&code=synthetic_code_123456",
        expired.marker.state
    );
    assert!(accept_callback(dir.path(), &cb).is_err());
    assert!(store.complete(&expired).is_err());
    drop(store);
    let mut reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.session.as_ref().unwrap().user.id, "fixture");
    assert_eq!(
        reopened.session.as_ref().unwrap().user.email,
        "updated@example.invalid"
    );
    reopened.clear().unwrap();
    drop(reopened);
    assert!(Store::open(dir.path()).unwrap().session.is_none());
    fs::write(dir.path().join("session.dat"), b"corrupted").unwrap();
    assert!(Store::open(dir.path()).is_err());
}
fn serve(response: String) -> (Api, std::thread::JoinHandle<String>) {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let n = s.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..at]).to_lowercase();
                let len = header
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= at + 4 + len {
                    break;
                }
            }
        }
        s.write_all(response.as_bytes()).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (Api::new(&base).unwrap(), handle)
}
fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
#[test]
fn account_http_contract_and_redacted_failures() {
    let body = serde_json::to_string(&fixture_session()).unwrap();
    let (api, server) = serve(ok(&body));
    let s = api.exchange("fixture_code", "fixture_verifier").unwrap();
    let req = server.join().unwrap();
    assert!(req.starts_with("POST /auth/token "));
    assert!(req.contains("\"verifier\":\"fixture_verifier\""));
    let (api, server) = serve(ok(
        r#"{"user":{"id":"fixture","email":"test@example.invalid","name":null}}"#,
    ));
    assert_eq!(api.refresh(&s.bearer()).unwrap().id, "fixture");
    assert!(
        server
            .join()
            .unwrap()
            .to_lowercase()
            .contains("authorization: bearer synthetic_token_not_a_real_secret")
    );
    let (api, server) = serve("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into());
    api.logout(&s.bearer()).unwrap();
    assert!(server.join().unwrap().starts_with("POST /auth/logout "));
    let(api,server)=serve("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret".into());
    assert_eq!(
        api.exchange("fixture", "fixture").err().unwrap(),
        "Account request failed (HTTP 302)"
    );
    server.join().unwrap();
    let (api, server) = serve(ok(&"x".repeat(LIMIT as usize + 1)));
    assert_eq!(
        api.refresh("fixture").err().unwrap(),
        "Account response exceeds limit"
    );
    server.join().unwrap();
    let mut expired = fixture_session();
    expired.expires_at = now() - 1;
    let (api, server) = serve(ok(&serde_json::to_string(&expired).unwrap()));
    assert!(api.exchange("fixture", "fixture").is_err());
    server.join().unwrap();
}
