//! Deterministic mutation fuzzing, with no extra dependency: every run checks the same cases.
//! `TESSEL_FUZZ_ITERATIONS=200000` makes a long run; the default keeps `cargo test` quick.
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor, Read};
use tessel_host_protocol::*;

const MARKER: &str = "zq-secret-marker";

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn iterations() -> usize {
    std::env::var("TESSEL_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000)
}

fn stream() -> StreamId {
    StreamId::from_bytes([0x5a; 16])
}

fn seeds() -> Vec<Vec<u8>> {
    let hello = Hello {
        versions: vec![1, 2],
        capabilities: vec![
            "pty".into(),
            "tree".into(),
            "git".into(),
            "future.optional".into(),
        ],
        budgets: Budgets::SLOW_HOST,
    };
    let client = [
        ClientFrame::request(1, ClientMessage::Hello(hello.clone())),
        ClientFrame::request(2, ClientMessage::Ping),
        ClientFrame::request(3, ClientMessage::Cancel(Cancel { target: 2 })),
        ClientFrame::request_raw(
            4,
            RawMessage::new("terminal.input", json!({ "text": MARKER, "data": "QUJD" })),
        ),
        ClientFrame::request_raw(
            5,
            RawMessage::new("file.read", json!({ "path": ["c3Jj", "bWFpbi5ycw=="] })),
        ),
        ClientFrame::ack(Ack {
            stream: stream(),
            seq: 9,
        }),
    ];
    let agreed = hello
        .negotiate(
            &Capability::ALL,
            Budgets::SLOW_HOST,
            &Epoch::from_bytes([7; 16]),
        )
        .unwrap();
    let host = [
        HostFrame::reply_ok(1, ReplyMessage::HelloOk(agreed).into_raw()),
        HostFrame::reply_error(
            2,
            WireError::detail(ErrorCode::Conflict, "changed by someone else"),
        ),
        HostFrame::connection_error(WireError::new(ErrorCode::BadFrame)),
        HostFrame::event(
            StreamPos {
                id: stream(),
                seq: 3,
            },
            RawMessage::new(
                "terminal.output",
                json!({ "data": "w6kbWzBtCg==", "note": MARKER }),
            ),
        ),
    ];
    let mut out: Vec<Vec<u8>> = client
        .iter()
        .map(|f| serde_json::to_vec(f).unwrap())
        .collect();
    out.extend(host.iter().map(|f| serde_json::to_vec(f).unwrap()));
    out
}

const SNIPPETS: &[&str] = &[
    "{",
    "}",
    "[",
    "]",
    "\"",
    ":",
    ",",
    "\\",
    "\\u0000",
    "{\"a\":",
    "[[[[[[[[[[[[[[[[[[",
    "null",
    "true",
    "-1",
    "1e400",
    "9007199254740992",
    "18446744073709551616",
    "\"version\":1,",
    "\"version\":2,",
    "\"ack\":{},",
    "\"kind\":\"PING\"",
    "\"zq-secret-marker\"",
    "\u{7f}",
    "\u{ff}",
];

fn mutate(rng: &mut Rng, bytes: &mut Vec<u8>) {
    for _ in 0..1 + rng.below(4) {
        let len = bytes.len();
        let at = rng.below(len + 1);
        match rng.below(7) {
            0 if len > 0 => bytes[at.min(len - 1)] ^= 1 << rng.below(8),
            1 if len > 0 => {
                let end = (at + 1 + rng.below(8)).min(len);
                bytes.drain(at.min(len)..end);
            }
            2 if len > 0 => {
                let end = (at + 1 + rng.below(16)).min(len);
                let copy = bytes[at.min(len)..end].to_vec();
                let to = rng.below(bytes.len() + 1);
                bytes.splice(to..to, copy);
            }
            3 => {
                let snippet = rng.pick(SNIPPETS).as_bytes().to_vec();
                bytes.splice(at..at, snippet);
            }
            4 => bytes.truncate(at),
            5 if len > 0 => {
                let at = at.min(len - 1);
                bytes[at] = *rng.pick(b"{}[]\":,0123456789-eE.\\ abcz\x00\xff");
            }
            _ => {
                if let Some(at) = bytes.iter().position(|&b| b == b'{') {
                    bytes.splice(at + 1..at + 1, b"\"version\":1,".iter().copied());
                }
            }
        }
    }
}

fn check_decode<T: Frame + PartialEq + std::fmt::Debug>(payload: &[u8]) {
    match decode_payload::<T>(payload) {
        Ok(frame) => {
            frame.check().unwrap();
            let bytes = encode_frame(&frame, MAX_FRAME_BYTES).unwrap();
            assert_eq!(decode_payload::<T>(&bytes[4..]).unwrap(), frame);
        }
        Err(error) => {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(wire_error(&error).is_some());
            assert!(!error.to_string().contains(MARKER));
        }
    }
}

#[test]
fn mutated_frames_never_panic_never_echo_and_round_trip_when_accepted() {
    let seeds = seeds();
    let mut rng = Rng::new(0x7e55e1);
    for _ in 0..iterations() {
        let mut bytes = rng.pick(&seeds).clone();
        mutate(&mut rng, &mut bytes);
        check_decode::<ClientFrame>(&bytes);
        check_decode::<HostFrame>(&bytes);
    }
}

struct Counting<R> {
    inner: R,
    read: usize,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buffer)?;
        self.read += n;
        Ok(n)
    }
}

#[test]
fn a_reader_never_reads_past_one_bounded_frame() {
    let seeds = seeds();
    let mut rng = Rng::new(0xf00d);
    for _ in 0..iterations() {
        let mut payload = rng.pick(&seeds).clone();
        if rng.below(2) == 0 {
            mutate(&mut rng, &mut payload);
        }
        let limit = *rng.pick(&[64usize, 4096, MAX_FRAME_BYTES, usize::MAX]);
        let length: u32 = match rng.below(4) {
            0 => rng.next() as u32,
            1 => (payload.len() as u32)
                .wrapping_add(rng.below(5) as u32)
                .wrapping_sub(2),
            _ => payload.len() as u32,
        };
        let mut bytes = length.to_be_bytes().to_vec();
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(b"trailing bytes of the next frame");
        let mut reader = Counting {
            inner: Cursor::new(bytes),
            read: 0,
        };
        let result = read_frame::<ClientFrame>(&mut reader, limit);
        assert!(
            reader.read <= 4 + limit.min(MAX_FRAME_BYTES),
            "read {} with limit {limit}",
            reader.read
        );
        if let Err(error) = result {
            assert!(!error.to_string().contains(MARKER));
        }
    }
}

const KINDS: &[&str] = &[
    "hello",
    "ping",
    "cancel",
    "terminal.create",
    "tree.subscribe",
    "git.status",
    "file.read",
    "future.thing",
    "tunnel.open",
];

fn random_frame(rng: &mut Rng, next_id: &mut u64) -> ClientFrame {
    if rng.below(8) == 0 {
        return ClientFrame::ack(Ack {
            stream: stream(),
            seq: 1 + rng.below(50) as u64,
        });
    }
    // Mostly increasing identifiers, sometimes a repeat or a step back.
    let id = match rng.below(12) {
        0 => next_id.saturating_sub(rng.below(3) as u64).max(1),
        _ => {
            *next_id += 1 + rng.below(2) as u64;
            *next_id
        }
    };
    let kind = *rng.pick(KINDS);
    let body = match kind {
        "hello" => serde_json::to_value(Hello {
            versions: rng.pick(&[vec![1u16], vec![2], vec![2, 1]]).clone(),
            capabilities: vec!["pty".into(), "tree".into(), "editor".into()],
            budgets: Budgets {
                in_flight: 1 + rng.below(4) as u16,
                ..Budgets::SLOW_HOST
            },
        })
        .unwrap(),
        "cancel" => json!({ "target": rng.below(id as usize + 2) }),
        "ping" if rng.below(4) == 0 => json!(1),
        _ => serde_json::Value::Null,
    };
    ClientFrame::request_raw(id, RawMessage::new(kind, body))
}

#[test]
fn a_connection_keeps_its_rules_under_any_sequence() {
    let mut rng = Rng::new(0xc0ffee);
    let limits = HostLimits {
        capabilities: vec![Capability::Pty, Capability::Tree],
        budgets: Budgets::SLOW_HOST,
        epoch: Epoch::from_bytes([9; 16]),
        info: HostInfo {
            version: "0.1.0".into(),
            arch: "x86_64".into(),
            pid: 42,
            persistence: Persistence::UNKNOWN,
        },
    };
    let mut connection = Connection::new(limits.clone());
    let mut next_id = 0u64;
    let mut waiting: Vec<u64> = Vec::new();
    let mut answered: HashSet<u64> = HashSet::new();
    for _ in 0..iterations() {
        if !waiting.is_empty() && rng.below(3) == 0 {
            let id = waiting.swap_remove(rng.below(waiting.len()));
            let reply = connection
                .finish(id, Ok(RawMessage::empty("done")))
                .expect("a dispatched request gets its reply");
            assert_eq!(reply.reply_to, Some(id));
            assert!(answered.insert(id));
            assert!(
                connection
                    .finish(id, Ok(RawMessage::empty("done")))
                    .is_none()
            );
            continue;
        }
        let frame = random_frame(&mut rng, &mut next_id);
        let before = connection.negotiated().is_some();
        let id = frame.request_id;
        match connection.on_frame(frame) {
            Ok(Step::Reply(reply)) => {
                assert_eq!(reply.reply_to, id);
                reply.check().unwrap();
                if !before {
                    assert_eq!(reply.ok.as_ref().map(|m| m.kind.as_str()), Some("hello_ok"));
                }
            }
            Ok(Step::Dispatch(dispatch)) => {
                assert!(before);
                assert!(
                    connection
                        .negotiated()
                        .unwrap()
                        .capabilities
                        .contains(&dispatch.capability)
                );
                waiting.push(dispatch.request_id);
            }
            Ok(Step::Ack(_)) => assert!(before),
            Err(fatal) => {
                fatal.frame().check().unwrap();
                connection = Connection::new(limits.clone());
                next_id = 0;
                waiting.clear();
                answered.clear();
            }
        }
        if let Some(agreed) = connection.negotiated() {
            assert!(connection.in_flight() <= usize::from(agreed.budgets.in_flight));
        }
    }
}

#[test]
fn streams_never_apply_twice_or_out_of_order() {
    let mut rng = Rng::new(0x5eed);
    for _ in 0..iterations() / 10 {
        let mut sender = StreamSender::new(1 + rng.below(4096) as u32);
        let mut receiver = StreamReceiver::default();
        let mut sent = Vec::new();
        let mut applied = Vec::new();
        for _ in 0..200 {
            match rng.below(3) {
                0 => {
                    if let Ok(seq) = sender.reserve(1 + rng.below(2048)) {
                        sent.push(seq);
                    }
                }
                1 => {
                    let seq = rng.below(sender.last_sent() as usize + 3) as u64;
                    let result = sender.ack(seq);
                    assert_eq!(result.is_err(), seq > sender.last_sent());
                }
                _ if !sent.is_empty() => {
                    // Delivery with duplicates and reordering.
                    let seq = *rng.pick(&sent);
                    if receiver.accept(seq) == Arrival::Next {
                        applied.push(seq);
                    }
                }
                _ => {}
            }
            assert!(sender.acked() <= sender.last_sent());
        }
        assert_eq!(applied, (1..=applied.len() as u64).collect::<Vec<_>>());
    }
}

#[test]
fn chunks_and_paths_from_any_text() {
    let mut rng = Rng::new(0xba5e64);
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=-_ .\\";
    for _ in 0..iterations() {
        let text: String = (0..rng.below(24))
            .map(|_| *rng.pick(alphabet) as char)
            .collect();
        if let Ok(chunk) = serde_json::from_value::<Chunk>(json!(text)) {
            assert_eq!(
                serde_json::to_value(&chunk).unwrap(),
                json!(text),
                "canonical: one spelling"
            );
        }
        let parts: Vec<String> = (0..rng.below(4))
            .map(|_| {
                (0..rng.below(12))
                    .map(|_| *rng.pick(alphabet) as char)
                    .collect()
            })
            .collect();
        if let Ok(path) = serde_json::from_value::<RelPath>(json!(parts)) {
            assert!(
                path.components()
                    .iter()
                    .all(|c| !c.is_empty() && c != b".." && !c.contains(&b'/'))
            );
            assert_eq!(
                serde_json::from_value::<RelPath>(serde_json::to_value(&path).unwrap()).unwrap(),
                path
            );
        }
    }
}

#[test]
fn deduplication_matches_a_simple_model() {
    let mut rng = Rng::new(0xdedf);
    let ttl = 100;
    let keys: Vec<DedupKey> = (0..6u8).map(|i| DedupKey::from_bytes([i; 16])).collect();
    let mut cache = DedupCache::<u64>::new(4, ttl);
    // key -> (finished at, outcome) or running
    let mut model: HashMap<usize, Option<(u64, u64)>> = HashMap::new();
    let mut now = 0u64;
    for _ in 0..iterations() {
        now += rng.below(30) as u64;
        model.retain(|_, entry| entry.is_none_or(|(at, _)| now - at < ttl));
        let k = rng.below(keys.len());
        match rng.below(3) {
            0 => {
                let result = cache.begin(&keys[k], now);
                match (model.get(&k), result) {
                    (None, Ok(Dedup::Fresh)) => {
                        model.insert(k, None);
                    }
                    (None, Err(error)) => {
                        assert_eq!(error.code, ErrorCode::Busy);
                        assert!(model.len() >= 4);
                    }
                    (Some(None), Ok(Dedup::Running)) => {}
                    (Some(Some((_, outcome))), Ok(Dedup::Finished(got))) => {
                        assert_eq!(*outcome, got)
                    }
                    (expected, got) => panic!("model {expected:?}, cache {got:?}"),
                }
            }
            1 => {
                if model.get(&k) == Some(&None) {
                    cache.finish(&keys[k], now, now);
                    model.insert(k, Some((now, now)));
                }
            }
            _ => {
                cache.abandon(&keys[k]);
                if model.get(&k) == Some(&None) {
                    model.remove(&k);
                }
            }
        }
        assert!(cache.len() <= 4);
    }
}
