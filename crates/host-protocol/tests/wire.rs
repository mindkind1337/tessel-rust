use serde_json::{Value, json};
use std::io::{self, Cursor, Read};
use tessel_host_protocol::*;

const EPOCH: &str = "0123456789abcdef0123456789abcdef";

fn epoch() -> Epoch {
    Epoch::parse(EPOCH).unwrap()
}

fn hello() -> Hello {
    Hello {
        versions: vec![VERSION],
        capabilities: vec!["tree".into(), "git".into(), "future.optional".into()],
        budgets: Budgets::SLOW_HOST,
    }
}

fn request() -> ClientFrame {
    ClientFrame::request(1, ClientMessage::Hello(hello()))
}

fn limits(capabilities: &[Capability]) -> HostLimits {
    HostLimits {
        capabilities: capabilities.to_vec(),
        budgets: Budgets::SLOW_HOST,
        epoch: epoch(),
    }
}

fn frame_bytes(json: &str) -> Vec<u8> {
    let mut bytes = (json.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(json.as_bytes());
    bytes
}

fn code_of(error: &io::Error) -> Option<ErrorCode> {
    wire_error(error).map(|e| e.code)
}

/// Each fixture parses, checks, and says the same thing once written again (compared as JSON
/// values: key order inside bodies is not part of the contract).
fn golden<T: Frame + std::fmt::Debug>(fixture: &str) -> T {
    let frame = decode_payload::<T>(fixture.trim().as_bytes()).unwrap();
    let again: Value =
        serde_json::from_slice(&encode_frame(&frame, MAX_FRAME_BYTES).unwrap()[4..]).unwrap();
    assert_eq!(again, serde_json::from_str::<Value>(fixture).unwrap());
    frame
}

#[test]
fn golden_vue_incremental_adoption_does_not_require_pty_or_tunnel() {
    let frame: ClientFrame = golden(include_str!("fixtures/hello-tree-git.json"));
    assert_eq!(frame, request());
    let ClientMessage::Hello(hello) = ClientMessage::from_raw(frame.message.as_ref().unwrap())
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    let response = hello
        .negotiate(
            &[Capability::Pty, Capability::Tree, Capability::Git],
            Budgets::SLOW_HOST,
            &epoch(),
        )
        .unwrap();
    assert_eq!(
        response.capabilities,
        vec![Capability::Tree, Capability::Git]
    );
    assert_eq!(response.budgets.total_ring_bytes, 16 * 1024 * 1024);
    let reply: HostFrame = golden(include_str!("fixtures/hello-ok.json"));
    assert_eq!(
        reply,
        HostFrame::reply_ok(1, ReplyMessage::HelloOk(response).into_raw())
    );
}

#[test]
fn golden_errors_events_acks_and_cancel() {
    let error: HostFrame = golden(include_str!("fixtures/reply-error.json"));
    assert_eq!(
        error,
        HostFrame::reply_error(7, WireError::new(ErrorCode::CapabilityNotNegotiated))
    );
    let fatal: HostFrame = golden(include_str!("fixtures/connection-error.json"));
    assert_eq!(
        fatal,
        HostFrame::connection_error(WireError::detail(
            ErrorCode::BadFrame,
            "request identifier must increase"
        ))
    );
    let event: HostFrame = golden(include_str!("fixtures/event.json"));
    let stream = event.stream.clone().unwrap();
    assert_eq!(stream.seq, 42);
    #[derive(serde::Deserialize)]
    struct Output {
        data: Chunk,
    }
    let output: Output = event.event.unwrap().body_as().unwrap();
    assert_eq!(output.data.as_bytes(), "é\u{1b}[0m\n".as_bytes());
    let ack: ClientFrame = golden(include_str!("fixtures/ack.json"));
    assert_eq!(
        ack,
        ClientFrame::ack(Ack {
            stream: stream.id,
            seq: 42
        })
    );
    let cancel: ClientFrame = golden(include_str!("fixtures/cancel.json"));
    assert_eq!(
        cancel,
        ClientFrame::request(9, ClientMessage::Cancel(Cancel { target: 8 }))
    );
}

#[test]
fn fragmented_frames_and_consecutive_messages_preserve_boundaries() {
    struct OneByte(Cursor<Vec<u8>>);
    impl Read for OneByte {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let size = buffer.len().min(1);
            self.0.read(&mut buffer[..size])
        }
    }
    let first = request();
    let second = ClientFrame::request(2, ClientMessage::Ping);
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &first, MAX_FRAME_BYTES).unwrap();
    write_frame(&mut bytes, &second, MAX_FRAME_BYTES).unwrap();
    let mut reader = OneByte(Cursor::new(bytes));
    assert_eq!(
        read_frame::<ClientFrame>(&mut reader, MAX_FRAME_BYTES).unwrap(),
        Some(first)
    );
    assert_eq!(
        read_frame::<ClientFrame>(&mut reader, MAX_FRAME_BYTES).unwrap(),
        Some(second)
    );
    assert_eq!(
        read_frame::<ClientFrame>(&mut reader, MAX_FRAME_BYTES).unwrap(),
        None
    );
}

#[test]
fn malformed_lengths_are_refused_before_reading_or_allocating_payload() {
    for length in [0u32, MAX_FRAME_BYTES as u32 + 1, u32::MAX] {
        let mut reader = Cursor::new(length.to_be_bytes());
        let error = read_frame::<ClientFrame>(&mut reader, MAX_FRAME_BYTES).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(code_of(&error), Some(ErrorCode::BadFrame));
        assert_eq!(reader.position(), 4);
    }
    // A smaller negotiated limit applies the same way.
    let mut reader = Cursor::new(frame_bytes(&serde_json::to_string(&request()).unwrap()));
    assert!(read_frame::<ClientFrame>(&mut reader, 64).is_err());
    assert_eq!(reader.position(), 4);
    for bytes in [vec![0], vec![0, 0], vec![0, 0, 0], vec![0, 0, 0, 3, b'{']] {
        let error =
            read_frame::<ClientFrame>(&mut Cursor::new(bytes), MAX_FRAME_BYTES).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}

#[test]
fn invalid_json_duplicate_fields_versions_and_ids_fail_without_echoing_payload() {
    let valid = serde_json::to_string(&request()).unwrap();
    let cases = [
        (valid.replace("\"request_id\":1", "\"request_id\":0"), ErrorCode::BadFrame),
        (valid.replace("\"request_id\":1", "\"request_id\":9007199254740992"), ErrorCode::BadFrame),
        (valid.replacen('{', "{\"version\":1,", 1), ErrorCode::BadFrame),
        (valid.replace("\"request_id\":1", "\"request_id\":1,\"secret\":\"must-not-log\""), ErrorCode::BadFrame),
        ("not-json-must-not-log".into(), ErrorCode::BadFrame),
        ("[".repeat(200), ErrorCode::BadFrame),
        (format!("{{\"a\":{}1{}}}", "[".repeat(MAX_JSON_DEPTH), "]".repeat(MAX_JSON_DEPTH)), ErrorCode::BadFrame),
        (valid.replace("\"kind\":\"hello\"", "\"kind\":\"Must-Not-Log\""), ErrorCode::BadFrame),
        (r#"{"version":1,"request_id":1}"#.into(), ErrorCode::BadFrame),
        (r#"{"version":1,"request_id":1,"message":{"kind":"ping"},"ack":{"stream":"0123456789abcdef0123456789abcdef","seq":1}}"#.into(), ErrorCode::BadFrame),
        (r#"{"version":1,"ack":{"stream":"0123456789ABCDEF0123456789ABCDEF","seq":1}}"#.into(), ErrorCode::BadFrame),
        (r#"{"version":1,"ack":{"stream":"0123456789abcdef0123456789abcdef","seq":0}}"#.into(), ErrorCode::BadFrame),
        (valid.replace("\"version\":1", "\"version\":2"), ErrorCode::UnsupportedVersion),
    ];
    for (json, code) in cases {
        let error =
            read_frame::<ClientFrame>(&mut Cursor::new(frame_bytes(&json)), MAX_FRAME_BYTES)
                .unwrap_err();
        assert_eq!(code_of(&error), Some(code), "{json}");
        assert!(!error.to_string().to_lowercase().contains("must-not-log"));
    }
    // Brackets inside strings are text, not nesting.
    let deep_text = ClientFrame::request(1, ClientMessage::Ping);
    let mut raw = deep_text.message.clone().unwrap();
    raw.kind = "terminal.input".into();
    raw.body = json!({ "text": "[".repeat(500) });
    assert!(
        decode_payload::<ClientFrame>(
            &encode_frame(&ClientFrame::request_raw(1, raw), MAX_FRAME_BYTES).unwrap()[4..]
        )
        .is_ok()
    );
    // Found by the fuzzer: a fraction or a huge number in a body does not mean the same once
    // sent again (nor in JavaScript). Integers within ±(2^53 − 1) only, anywhere in a body.
    for body in [
        json!({ "n": 1.5 }),
        json!({ "n": 1e28 }),
        json!([9007199254740992u64]),
        json!({ "a": { "b": [-9007199254740992i64] } }),
    ] {
        let frame = ClientFrame::request_raw(1, RawMessage::new("terminal.resize", body));
        assert_eq!(frame.check().unwrap_err().code, ErrorCode::BadFrame);
        let json = serde_json::to_string(&frame).unwrap();
        assert!(
            read_frame::<ClientFrame>(&mut Cursor::new(frame_bytes(&json)), MAX_FRAME_BYTES)
                .is_err()
        );
    }
    let edge = json!({ "max": 9007199254740991u64, "min": -9007199254740991i64, "zero": 0 });
    assert!(
        ClientFrame::request_raw(1, RawMessage::new("terminal.resize", edge))
            .check()
            .is_ok()
    );
}

#[test]
fn negotiated_limits_only_decrease_and_unsupported_versions_fail() {
    let mut hello = hello();
    let smaller = Budgets {
        frame_bytes: 4096,
        terminal_ring_bytes: 1024,
        total_ring_bytes: 4096,
        stream_window_bytes: 8192,
        in_flight: 4,
    };
    let result = hello
        .negotiate(&[Capability::Git, Capability::Git], smaller, &epoch())
        .unwrap();
    assert_eq!(result.budgets, smaller);
    assert_eq!(result.capabilities, vec![Capability::Git]);
    hello.versions = vec![2];
    assert_eq!(
        hello.negotiate(&[], smaller, &epoch()).unwrap_err().code,
        ErrorCode::UnsupportedVersion
    );
    hello.versions = vec![2, 1];
    assert_eq!(hello.negotiate(&[], smaller, &epoch()).unwrap().version, 1);
    for broken in [
        Budgets {
            terminal_ring_bytes: Budgets::SLOW_HOST.total_ring_bytes + 1,
            ..Budgets::SLOW_HOST
        },
        Budgets {
            stream_window_bytes: Budgets::SLOW_HOST.frame_bytes - 1,
            ..Budgets::SLOW_HOST
        },
        Budgets {
            in_flight: 0,
            ..Budgets::SLOW_HOST
        },
        Budgets {
            frame_bytes: 1023,
            ..Budgets::SLOW_HOST
        },
        Budgets {
            total_ring_bytes: 64 * 1024 * 1024 + 1,
            ..Budgets::SLOW_HOST
        },
    ] {
        hello.budgets = broken;
        assert!(hello.validate().is_err());
    }
}

#[test]
fn oversized_outbound_frame_never_writes_a_partial_header() {
    let mut bytes = Vec::new();
    assert!(write_frame(&mut bytes, &request(), 8).is_err());
    assert!(bytes.is_empty());
    let mut message = request();
    message.request_id = Some(u64::MAX);
    assert!(write_frame(&mut bytes, &message, MAX_FRAME_BYTES).is_err());
    let error = WireError {
        detail: Some("not\nprintable".into()),
        ..WireError::new(ErrorCode::Internal)
    };
    assert!(
        write_frame(
            &mut bytes,
            &HostFrame::reply_error(1, error),
            MAX_FRAME_BYTES
        )
        .is_err()
    );
    assert!(bytes.is_empty());
}

#[test]
fn chunks_are_canonical_base64_within_64_kib() {
    let chunk = |text: &str| serde_json::from_value::<Chunk>(json!(text));
    assert_eq!(chunk("QQ==").unwrap().as_bytes(), b"A");
    for bad in ["QQ", "QR==", "Q===", "QQ==QQ==", "Q Q=", "QQ-_"] {
        assert!(chunk(bad).is_err(), "{bad}");
    }
    let full = Chunk::new(vec![7; MAX_CHUNK_BYTES]).unwrap();
    let text = serde_json::to_value(&full).unwrap();
    assert_eq!(serde_json::from_value::<Chunk>(text).unwrap(), full);
    assert!(Chunk::new(vec![0; MAX_CHUNK_BYTES + 1]).is_none());
    // 65,541 bytes once decoded: refused by its length, before decoding.
    assert!(chunk(&"AAAA".repeat(MAX_CHUNK_BYTES / 3 + 2)).is_err());
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn paths_keep_raw_names_and_refuse_escapes() {
    let path = RelPath::from_components(vec![
        b"src".to_vec(),
        vec![0xff, 0xfe, b'x'],
        "é".as_bytes().to_vec(),
    ])
    .unwrap();
    let text = serde_json::to_value(&path).unwrap();
    assert_eq!(serde_json::from_value::<RelPath>(text).unwrap(), path);
    assert_eq!(
        path.to_bytes(),
        [
            b"src/".as_slice(),
            &[0xff, 0xfe, b'x', b'/'],
            "é".as_bytes()
        ]
        .concat()
    );
    assert!(
        serde_json::from_value::<RelPath>(json!([]))
            .unwrap()
            .is_root()
    );
    for bad in [
        vec![b"..".to_vec()],
        vec![b".".to_vec()],
        vec![b"a/b".to_vec()],
        vec![b"a\0".to_vec()],
        vec![vec![]],
        vec![vec![b'a'; 256]],
    ] {
        assert!(RelPath::from_components(bad.clone()).is_none(), "{bad:?}");
        let text: Vec<String> = bad
            .iter()
            .map(|c| {
                if c.is_empty() {
                    String::new()
                } else {
                    base64_of(c)
                }
            })
            .collect();
        assert!(serde_json::from_value::<RelPath>(json!(text)).is_err());
    }
    let long = vec![vec![b'a'; 255]; 17];
    assert!(RelPath::from_components(long).is_none());
    assert!(RelPath::from_components(vec![b"a".to_vec(); MAX_PATH_COMPONENTS + 1]).is_none());
}

#[test]
fn ids_have_one_spelling() {
    assert!(StreamId::parse("0123456789abcdef0123456789abcdef").is_some());
    for bad in [
        "0123456789ABCDEF0123456789ABCDEF",
        "0123",
        "g123456789abcdef0123456789abcdef",
        "",
    ] {
        assert!(StreamId::parse(bad).is_none());
        assert!(DedupKey::parse(bad).is_none());
    }
    assert_eq!(StreamId::from_bytes([0xab; 16]).as_str(), "ab".repeat(16));
}

#[test]
fn unknown_error_codes_from_a_newer_host_are_kept_as_unknown() {
    let frame: HostFrame = serde_json::from_str(
        r#"{"version":1,"reply_to":3,"error":{"code":"quota_exceeded","retryable":false}}"#,
    )
    .unwrap();
    assert_eq!(frame.error.unwrap().code, ErrorCode::Unknown);
    assert!(WireError::new(ErrorCode::Busy).retryable);
    assert!(!WireError::new(ErrorCode::Conflict).retryable);
}

fn connected(capabilities: &[Capability]) -> Connection {
    let mut connection = Connection::new(limits(capabilities));
    let hello = ClientFrame::request(
        1,
        ClientMessage::Hello(Hello {
            capabilities: vec!["pty".into(), "tree".into(), "git".into(), "editor".into()],
            budgets: Budgets {
                in_flight: 2,
                ..Budgets::SLOW_HOST
            },
            ..hello()
        }),
    );
    assert!(matches!(connection.on_frame(hello), Ok(Step::Reply(_))));
    connection
}

fn raw(id: u64, kind: &str) -> ClientFrame {
    ClientFrame::request_raw(id, RawMessage::empty(kind))
}

fn reply_code(step: Result<Step, Fatal>) -> Option<ErrorCode> {
    match step {
        Ok(Step::Reply(frame)) => frame.error.map(|e| e.code),
        other => panic!("expected a reply, got {other:?}"),
    }
}

/// The error and compatibility matrix of PROTOCOL.md, row by row.
#[test]
fn compatibility_matrix() {
    // Version 2 only: refused with a reply to that hello, then closed.
    let mut c = Connection::new(limits(&[Capability::Tree]));
    let fatal = c
        .on_frame(ClientFrame::request(
            1,
            ClientMessage::Hello(Hello {
                versions: vec![2],
                ..hello()
            }),
        ))
        .unwrap_err();
    assert_eq!(
        (fatal.reply_to, fatal.error.code),
        (Some(1), ErrorCode::UnsupportedVersion)
    );
    // Versions [2, 1]: version 1.
    let mut c = Connection::new(limits(&[Capability::Tree]));
    let Ok(Step::Reply(ok)) = c.on_frame(ClientFrame::request(
        1,
        ClientMessage::Hello(Hello {
            versions: vec![2, 1],
            ..hello()
        }),
    )) else {
        panic!()
    };
    let Some(ReplyMessage::HelloOk(agreed)) =
        ReplyMessage::from_raw(ok.ok.as_ref().unwrap()).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (agreed.version, agreed.capabilities),
        (1, vec![Capability::Tree])
    );
    // An envelope of another version: closed, about the connection.
    let mut c = Connection::new(limits(&[]));
    let mut frame = request();
    frame.version = 2;
    let fatal = c.on_frame(frame).unwrap_err();
    assert_eq!(
        (fatal.reply_to, fatal.error.code),
        (None, ErrorCode::UnsupportedVersion)
    );
    // Anything before hello: closed.
    let mut c = Connection::new(limits(&[Capability::Pty]));
    assert_eq!(
        c.on_frame(raw(1, "ping")).unwrap_err().error.code,
        ErrorCode::NotNegotiated
    );
    let mut c = Connection::new(limits(&[Capability::Pty]));
    assert_eq!(
        c.on_frame(raw(1, "terminal.create"))
            .unwrap_err()
            .error
            .code,
        ErrorCode::NotNegotiated
    );
    let mut c = Connection::new(limits(&[Capability::Pty]));
    let ack = ClientFrame::ack(Ack {
        stream: StreamId::from_bytes([1; 16]),
        seq: 1,
    });
    assert_eq!(
        c.on_frame(ack.clone()).unwrap_err().error.code,
        ErrorCode::NotNegotiated
    );
    // A malformed hello: closed with a reply to it.
    let mut c = Connection::new(limits(&[]));
    let fatal = c
        .on_frame(ClientFrame::request_raw(
            1,
            RawMessage::new("hello", json!({"versions": [1]})),
        ))
        .unwrap_err();
    assert_eq!(
        (fatal.reply_to, fatal.error.code),
        (Some(1), ErrorCode::BadRequest)
    );

    // After hello, refusals are replies and the connection goes on.
    let mut c = connected(&[Capability::Pty, Capability::Tree]);
    assert_eq!(
        reply_code(c.on_frame(raw(2, "future.thing"))),
        Some(ErrorCode::UnsupportedKind)
    );
    assert_eq!(
        reply_code(c.on_frame(raw(3, "git.status"))),
        Some(ErrorCode::CapabilityNotNegotiated)
    );
    assert_eq!(
        reply_code(c.on_frame(raw(4, "hello"))),
        Some(ErrorCode::BadRequest)
    );
    assert_eq!(
        reply_code(c.on_frame(ClientFrame::request_raw(
            5,
            RawMessage::new("ping", json!(1))
        ))),
        Some(ErrorCode::BadRequest)
    );
    assert_eq!(reply_code(c.on_frame(raw(6, "ping"))), None);
    assert!(matches!(c.on_frame(ack), Ok(Step::Ack(_))));
    let Ok(Step::Dispatch(dispatch)) = c.on_frame(raw(7, "tree.subscribe")) else {
        panic!()
    };
    assert_eq!(
        (dispatch.request_id, dispatch.capability),
        (7, Capability::Tree)
    );
    assert!(matches!(
        c.on_frame(raw(8, "terminal.create")),
        Ok(Step::Dispatch(_))
    ));
    // in_flight 2: the third waits as busy, which is retryable.
    let Ok(Step::Reply(busy)) = c.on_frame(raw(9, "terminal.list")) else {
        panic!()
    };
    assert!(busy.error.as_ref().unwrap().retryable);
    assert_eq!(busy.error.unwrap().code, ErrorCode::Busy);
    // Cancel: done for one in flight, not found for one answered or unknown, and the target
    // gets `cancelled` as its one reply.
    let cancel = |id, target| ClientFrame::request(id, ClientMessage::Cancel(Cancel { target }));
    assert_eq!(reply_code(c.on_frame(cancel(10, 7))), None);
    assert_eq!(
        reply_code(c.on_frame(cancel(11, 7))),
        Some(ErrorCode::NotFound)
    );
    assert_eq!(
        reply_code(c.on_frame(cancel(12, 6))),
        Some(ErrorCode::NotFound)
    );
    assert_eq!(
        reply_code(c.on_frame(cancel(13, 13))),
        Some(ErrorCode::BadRequest)
    );
    let first = c.finish(7, Ok(RawMessage::empty("done"))).unwrap();
    assert_eq!(first.error.unwrap().code, ErrorCode::Cancelled);
    assert!(c.finish(7, Ok(RawMessage::empty("done"))).is_none());
    assert_eq!(
        c.finish(8, Err(WireError::new(ErrorCode::NotFound)))
            .unwrap()
            .error
            .unwrap()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(c.in_flight(), 0);
    // Identifiers must go up: closed.
    assert_eq!(
        c.on_frame(raw(13, "ping")).unwrap_err().error.code,
        ErrorCode::BadFrame
    );
}

#[test]
fn streams_apply_each_event_once_and_bound_unacknowledged_bytes() {
    let mut receiver = StreamReceiver::default();
    assert_eq!(receiver.accept(1), Arrival::Next);
    assert_eq!(receiver.accept(1), Arrival::Duplicate);
    assert_eq!(receiver.accept(3), Arrival::Gap { expected: 2 });
    assert_eq!(receiver.accept(2), Arrival::Next);
    assert_eq!(
        StreamReceiver::resume_after(10).accept(10),
        Arrival::Duplicate
    );

    let mut sender = StreamSender::new(100);
    assert_eq!(sender.reserve(60), Ok(1));
    assert_eq!(sender.reserve(40), Ok(2));
    assert_eq!(sender.reserve(1), Err(SendError::WindowFull));
    assert_eq!(sender.ack(3), Err(AckError::Unsent));
    sender.ack(1).unwrap();
    assert_eq!(sender.unacked_bytes(), 40);
    sender.ack(1).unwrap();
    assert_eq!(sender.reserve(60), Ok(3));
    sender.ack(3).unwrap();
    assert_eq!(sender.unacked_bytes(), 0);
    // Nothing in flight: one event larger than the window still goes.
    assert_eq!(sender.reserve(1000), Ok(4));
}

#[test]
fn deduplication_never_repeats_an_effect_within_its_time() {
    let key = DedupKey::from_bytes([3; 16]);
    let mut cache = DedupCache::<u32>::new(2, 1000);
    assert_eq!(cache.begin(&key, 0).unwrap(), Dedup::Fresh);
    assert_eq!(cache.begin(&key, 1).unwrap(), Dedup::Running);
    cache.finish(&key, 41, 10);
    assert_eq!(cache.begin(&key, 500).unwrap(), Dedup::Finished(41));
    let other = DedupKey::from_bytes([4; 16]);
    let third = DedupKey::from_bytes([5; 16]);
    assert_eq!(cache.begin(&other, 600).unwrap(), Dedup::Fresh);
    assert_eq!(cache.begin(&third, 700).unwrap_err().code, ErrorCode::Busy);
    // Finished entries leave after their time; running ones never do.
    assert_eq!(cache.begin(&third, 5000).unwrap(), Dedup::Fresh);
    assert_eq!(cache.begin(&other, 99_999).unwrap(), Dedup::Running);
    cache.abandon(&other);
    assert_eq!(cache.begin(&other, 100_000).unwrap(), Dedup::Fresh);
}
