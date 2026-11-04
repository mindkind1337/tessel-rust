use std::io::{self, Cursor, Read};
use tessel_host_protocol::*;

fn request() -> Envelope {
    Envelope {
        version: VERSION,
        request_id: 1,
        message: Message::Hello(Hello {
            versions: vec![VERSION],
            capabilities: vec!["tree".into(), "git".into(), "future.optional".into()],
            budgets: Budgets::SLOW_HOST,
        }),
    }
}

#[test]
fn golden_vue_incremental_adoption_does_not_require_pty_or_tunnel() {
    let fixture = include_str!("fixtures/hello-tree-git.json").trim();
    let message: Envelope = serde_json::from_str(fixture).unwrap();
    assert_eq!(message, request());
    assert_eq!(serde_json::to_string(&message).unwrap(), fixture);
    let Message::Hello(hello) = message.message else {
        panic!()
    };
    let response = hello
        .negotiate(
            &[Capability::Pty, Capability::Tree, Capability::Git],
            Budgets::SLOW_HOST,
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
    assert_eq!(
        response.capabilities,
        vec![Capability::Tree, Capability::Git]
    );
    assert_eq!(response.budgets.total_ring_bytes, 16 * 1024 * 1024);
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
    let second = Envelope {
        version: VERSION,
        request_id: 2,
        message: Message::Ping,
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &first, MAX_FRAME_BYTES).unwrap();
    write_frame(&mut bytes, &second, MAX_FRAME_BYTES).unwrap();
    let mut reader = OneByte(Cursor::new(bytes));
    assert_eq!(
        read_frame(&mut reader, MAX_FRAME_BYTES).unwrap(),
        Some(first)
    );
    assert_eq!(
        read_frame(&mut reader, MAX_FRAME_BYTES).unwrap(),
        Some(second)
    );
    assert_eq!(read_frame(&mut reader, MAX_FRAME_BYTES).unwrap(), None);
}

#[test]
fn malformed_lengths_are_refused_before_reading_or_allocating_payload() {
    for length in [0u32, MAX_FRAME_BYTES as u32 + 1, u32::MAX] {
        let mut reader = Cursor::new(length.to_be_bytes());
        assert_eq!(
            read_frame(&mut reader, MAX_FRAME_BYTES).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(reader.position(), 4);
    }
    for bytes in [vec![0], vec![0, 0], vec![0, 0, 0], vec![0, 0, 0, 3, b'{']] {
        assert_eq!(
            read_frame(&mut Cursor::new(bytes), MAX_FRAME_BYTES)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}

#[test]
fn invalid_json_duplicate_fields_versions_and_ids_fail_without_echoing_payload() {
    let valid = serde_json::to_string(&request()).unwrap();
    for json in [
        valid.replace("\"version\":1", "\"version\":2"),
        valid.replace("\"request_id\":1", "\"request_id\":0"),
        valid.replace("\"request_id\":1", "\"request_id\":9007199254740992"),
        valid.replacen("{", "{\"version\":1,", 1),
        valid.replace(
            "\"request_id\":1",
            "\"request_id\":1,\"secret\":\"must-not-log\"",
        ),
        "not-json-must-not-log".into(),
        "[".repeat(200),
    ] {
        let mut bytes = (json.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(json.as_bytes());
        let error = read_frame(&mut Cursor::new(bytes), MAX_FRAME_BYTES).unwrap_err();
        assert!(!error.to_string().contains("must-not-log"));
    }
}

#[test]
fn negotiated_limits_only_decrease_and_unsupported_versions_fail() {
    let Message::Hello(mut hello) = request().message else {
        panic!()
    };
    let smaller = Budgets {
        frame_bytes: 4096,
        terminal_ring_bytes: 1024,
        total_ring_bytes: 4096,
    };
    let result = hello
        .negotiate(
            &[Capability::Git, Capability::Git],
            smaller,
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
    assert_eq!(result.budgets, smaller);
    assert_eq!(result.capabilities, vec![Capability::Git]);
    hello.versions = vec![2];
    assert!(
        hello
            .negotiate(&[], smaller, "0123456789abcdef0123456789abcdef")
            .is_err()
    );
    hello.versions = vec![1];
    hello.budgets.terminal_ring_bytes = hello.budgets.total_ring_bytes + 1;
    assert!(hello.validate().is_err());
}

#[test]
fn oversized_outbound_frame_never_writes_a_partial_header() {
    let mut bytes = Vec::new();
    assert!(write_frame(&mut bytes, &request(), 8).is_err());
    assert!(bytes.is_empty());
    let mut message = request();
    message.request_id = u64::MAX;
    assert!(write_frame(&mut bytes, &message, MAX_FRAME_BYTES).is_err());
    assert!(bytes.is_empty());
}

#[test]
fn connection_requires_hello_and_rejects_duplicates_and_response_injection() {
    let mut state = Handshake::new();
    let epoch = "0123456789abcdef0123456789abcdef";
    let ping = Envelope {
        version: 1,
        request_id: 2,
        message: Message::Ping,
    };
    assert!(
        state
            .accept(ping.clone(), &[], Budgets::SLOW_HOST, epoch)
            .is_err()
    );
    assert!(state.negotiated().is_none());
    let response = state
        .accept(request(), &[Capability::Git], Budgets::SLOW_HOST, epoch)
        .unwrap();
    assert!(matches!(response.message, Message::HelloOk(_)));
    assert!(
        state
            .accept(request(), &[], Budgets::SLOW_HOST, epoch)
            .is_err()
    );
    assert!(
        state
            .accept(
                Envelope {
                    request_id: 3,
                    ..response
                },
                &[],
                Budgets::SLOW_HOST,
                epoch
            )
            .is_err()
    );
    assert_eq!(
        state
            .accept(ping.clone(), &[], Budgets::SLOW_HOST, epoch)
            .unwrap()
            .message,
        Message::Pong
    );
    assert!(state.accept(ping, &[], Budgets::SLOW_HOST, epoch).is_err());
}
