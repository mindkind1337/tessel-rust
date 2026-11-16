//! Bounded MCP handshake protocol only. No credentials, listener, or tool execution.
//! The caller must authenticate and scope every dispatched operation separately.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_PENDING: usize = 16;
const VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// Read one complete newline-delimited frame without unbounded allocation.
/// Oversize and unterminated frames require closing the transport.
pub fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut frame)?;
    if frame.is_empty() {
        return Ok(None);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MCP frame exceeds limit",
        ));
    }
    if frame.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Incomplete MCP frame",
        ));
    }
    Ok(Some(frame))
}

struct BoundedOutput(Vec<u8>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > (MAX_FRAME_BYTES - 1).saturating_sub(self.0.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP response exceeds limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_json(value: &Value) -> io::Result<Vec<u8>> {
    let mut encoded = BoundedOutput(Vec::new());
    serde_json::to_writer(&mut encoded, value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "MCP response cannot be encoded within limit",
        )
    })?;
    Ok(encoded.0)
}

/// Buffer within the cap before writing, so oversized output emits no partial JSON.
pub fn write_message(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let mut encoded = encode_json(value)?;
    encoded.push(b'\n');
    writer.write_all(&encoded)?;
    writer.flush()
}

#[derive(Debug, PartialEq)]
pub enum Operation {
    List,
    Call { name: String, arguments: Value },
}

#[derive(Debug, PartialEq)]
pub enum Route {
    Reply(Value),
    Dispatch {
        id: Value,
        ticket: u64,
        operation: Operation,
    },
    Cancel {
        id: Value,
        ticket: u64,
    },
    Ignore,
}

#[derive(Debug)]
pub enum BackendError {
    UnknownTool,
    Failed(String),
}

#[derive(Default, PartialEq, Eq)]
enum Phase {
    #[default]
    New,
    AwaitReady,
    Ready,
}

enum PendingKind {
    List,
    Call,
}

#[derive(Default)]
pub struct Session {
    phase: Phase,
    pending: HashMap<String, (u64, PendingKind)>,
    next_ticket: u64,
}

fn id_key(id: &Value) -> Option<String> {
    match id {
        Value::String(text) if text.len() <= 128 => Some(format!("s:{text}")),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(format!("n:{number}")),
        _ => None,
    }
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "result":result})
}

fn oversized_result(id: &Value) -> Value {
    failure(
        id.clone(),
        -32000,
        "Response exceeds limit; the operation may already have completed. Do not retry automatically.",
    )
}

impl Session {
    pub fn receive(&mut self, bytes: &[u8]) -> Route {
        if bytes.len() > MAX_FRAME_BYTES {
            return Route::Reply(failure(Value::Null, -32600, "Request exceeds limit"));
        }
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            return Route::Reply(failure(Value::Null, -32700, "Invalid JSON"));
        };
        self.route(value)
    }

    fn route(&mut self, value: Value) -> Route {
        let invalid = || Route::Reply(failure(Value::Null, -32600, "Invalid request"));
        let Some(object) = value.as_object() else {
            return invalid();
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return invalid();
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return invalid();
        };
        if method.len() > 128 || object.contains_key("result") || object.contains_key("error") {
            return invalid();
        }
        let id = object.get("id");
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        if id.is_none() {
            if !params.is_object() {
                return Route::Ignore;
            }
            match method {
                "notifications/initialized" if self.phase == Phase::AwaitReady => {
                    self.phase = Phase::Ready
                }
                "notifications/cancelled" if self.phase == Phase::Ready => {
                    if let Some(id) = params.get("requestId")
                        && let Some(key) = id_key(id)
                        && let Some((ticket, _)) = self.pending.remove(&key)
                    {
                        return Route::Cancel {
                            id: id.clone(),
                            ticket,
                        };
                    }
                }
                _ => {}
            }
            return Route::Ignore;
        }
        let id = id.unwrap().clone();
        let Some(key) = id_key(&id) else {
            return invalid();
        };
        if self.pending.contains_key(&key) {
            return Route::Reply(failure(id, -32600, "Request id already pending"));
        }
        if !params.is_object() {
            return Route::Reply(failure(id, -32602, "Parameters must be an object"));
        }
        if method == "ping" {
            return Route::Reply(success(id, json!({})));
        }
        if method == "initialize" {
            if self.phase != Phase::New {
                return Route::Reply(failure(id, -32600, "Already initialized"));
            }
            let Some(version) = params.get("protocolVersion").and_then(Value::as_str) else {
                return Route::Reply(failure(id, -32602, "Missing protocol version"));
            };
            let client = params.get("clientInfo");
            if version.len() > 64
                || !params.get("capabilities").is_some_and(Value::is_object)
                || !["name", "version"].iter().all(|field| {
                    client
                        .and_then(|c| c.get(field))
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty() && s.len() <= 256)
                })
            {
                return Route::Reply(failure(id, -32602, "Invalid initialization parameters"));
            }
            let version = if VERSIONS.contains(&version) {
                version
            } else {
                "2025-11-25"
            };
            self.phase = Phase::AwaitReady;
            return Route::Reply(success(
                id,
                json!({
                    "protocolVersion":version,
                    "capabilities":{"tools":{"listChanged":false}},
                    "serverInfo":{"name":"tessel-team", "version":env!("CARGO_PKG_VERSION")}
                }),
            ));
        }
        if self.phase != Phase::Ready {
            return Route::Reply(failure(id, -32000, "MCP initialization is not complete"));
        }
        let (kind, operation) = match method {
            "tools/list" => {
                if params.get("cursor").is_some() {
                    return Route::Reply(failure(id, -32602, "No pagination cursor is supported"));
                }
                (PendingKind::List, Operation::List)
            }
            "tools/call" => {
                let Some(name) = params
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty() && name.len() <= 128)
                else {
                    return Route::Reply(failure(id, -32602, "Invalid tool name"));
                };
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if !arguments.is_object() {
                    return Route::Reply(failure(id, -32602, "Tool arguments must be an object"));
                }
                (
                    PendingKind::Call,
                    Operation::Call {
                        name: name.into(),
                        arguments,
                    },
                )
            }
            _ => return Route::Reply(failure(id, -32601, "Method not found")),
        };
        if self.pending.len() >= MAX_PENDING {
            return Route::Reply(failure(id, -32000, "Too many pending requests"));
        }
        let Some(ticket) = self.next_ticket.checked_add(1) else {
            return Route::Reply(failure(id, -32000, "Request sequence exhausted"));
        };
        self.next_ticket = ticket;
        self.pending.insert(key, (ticket, kind));
        Route::Dispatch {
            id,
            ticket,
            operation,
        }
    }

    /// A cancelled or already completed request cannot emit a later result.
    pub fn complete(
        &mut self,
        id: &Value,
        ticket: u64,
        result: Result<Value, BackendError>,
    ) -> Option<Value> {
        let key = id_key(id)?;
        if self.pending.get(&key)?.0 != ticket {
            return None;
        }
        let (_, kind) = self.pending.remove(&key)?;
        let response = match result {
            Err(BackendError::UnknownTool) => failure(id.clone(), -32602, "Unknown tool"),
            Err(BackendError::Failed(message)) => match kind {
                PendingKind::List => failure(id.clone(), -32000, &message),
                PendingKind::Call => success(
                    id.clone(),
                    json!({"content":[{"type":"text","text":message}],"isError":true}),
                ),
            },
            Ok(value) => match kind {
                PendingKind::List if value.is_array() => {
                    success(id.clone(), json!({"tools":value}))
                }
                PendingKind::List => failure(id.clone(), -32603, "Invalid tool catalogue"),
                PendingKind::Call => match encode_json(&value)
                    .and_then(|encoded| String::from_utf8(encoded).map_err(io::Error::other))
                {
                    Ok(text) => success(
                        id.clone(),
                        json!({"content":[{"type":"text","text":text}],"isError":false}),
                    ),
                    Err(_) => oversized_result(id),
                },
            },
        };
        // Nested JSON text may expand again when escaped in the MCP envelope.
        Some(if encode_json(&response).is_ok() {
            response
        } else {
            oversized_result(id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn send(session: &mut Session, value: Value) -> Route {
        session.receive(&serde_json::to_vec(&value).unwrap())
    }
    fn request(id: Value, method: &str, params: Value) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    }
    fn init(version: &str) -> Value {
        request(
            json!(0),
            "initialize",
            json!({"protocolVersion":version,
            "capabilities":{},"clientInfo":{"name":"synthetic-client","version":"1"}}),
        )
    }
    fn ready() -> Session {
        let mut session = Session::default();
        assert!(matches!(
            send(&mut session, init("2025-11-25")),
            Route::Reply(_)
        ));
        assert_eq!(
            send(
                &mut session,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            ),
            Route::Ignore
        );
        session
    }
    fn reply(route: Route) -> Value {
        match route {
            Route::Reply(value) => value,
            _ => panic!("expected reply"),
        }
    }
    fn ticket(route: Route) -> (Value, u64, Operation) {
        match route {
            Route::Dispatch {
                id,
                ticket,
                operation,
            } => (id, ticket, operation),
            _ => panic!("expected dispatch"),
        }
    }

    #[test]
    fn frames_are_bounded_complete_and_stdout_contains_only_json() {
        let mut output = Vec::new();
        let message = json!({"text":"étiquette\nnext line", "id":1});
        write_message(&mut output, &message).unwrap();
        assert_eq!(output.iter().filter(|&&b| b == b'\n').count(), 1);
        let mut input = Cursor::new(output);
        let frame = read_frame(&mut input).unwrap().unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&frame).unwrap(), message);
        assert!(read_frame(&mut input).unwrap().is_none());
        let mut input = Cursor::new(b"{}\r\n{}\n");
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"{}\r\n");
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"{}\n");
        assert!(read_frame(&mut Cursor::new(b"{}")).is_err());
        let mut input = Cursor::new(vec![b'x'; MAX_FRAME_BYTES * 2]);
        assert!(read_frame(&mut input).is_err());
        assert_eq!(input.position(), (MAX_FRAME_BYTES + 1) as u64);
        let mut output = Vec::new();
        assert!(write_message(&mut output, &json!("x".repeat(MAX_FRAME_BYTES))).is_err());
        assert!(
            output.is_empty(),
            "oversized output must not partially write JSON"
        );
    }

    #[test]
    fn handshake_negotiates_only_supported_versions_and_gates_dispatch() {
        for version in VERSIONS.into_iter().chain(["2099-01-01"]) {
            let mut session = Session::default();
            assert!(
                reply(send(
                    &mut session,
                    request(json!(1), "tools/list", json!({}))
                ))["error"]
                    .is_object()
            );
            assert_eq!(
                reply(send(&mut session, request(json!(2), "ping", json!({}))))["result"],
                json!({})
            );
            let result = reply(send(&mut session, init(version)));
            assert_eq!(
                result["result"]["protocolVersion"],
                if VERSIONS.contains(&version) {
                    version
                } else {
                    "2025-11-25"
                }
            );
            assert_eq!(
                result["result"]["capabilities"],
                json!({"tools":{"listChanged":false}})
            );
            assert!(
                reply(send(
                    &mut session,
                    request(json!(3), "tools/list", json!({}))
                ))["error"]
                    .is_object()
            );
            send(
                &mut session,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            );
            assert!(matches!(
                send(&mut session, request(json!(4), "tools/list", json!({}))),
                Route::Dispatch { .. }
            ));
            assert!(reply(send(&mut session, init(version)))["error"].is_object());
        }
    }

    #[test]
    fn invalid_messages_and_notifications_never_dispatch_tools() {
        let mut session = ready();
        for invalid in [
            json!([]),
            json!({}),
            request(Value::Null, "tools/list", json!({})),
            request(json!(1.5), "tools/list", json!({})),
            request(json!(true), "tools/list", json!({})),
            request(json!("x".repeat(129)), "tools/list", json!({})),
            request(
                json!(1),
                "tools/call",
                json!({"name":"tool","arguments":[]}),
            ),
            request(json!(1), "tools/list", json!({"cursor":"not-issued"})),
            request(json!(1), "tools/list", json!([])),
        ] {
            assert!(reply(send(&mut session, invalid))["error"].is_object());
        }
        assert_eq!(
            send(
                &mut session,
                json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"team_create"}})
            ),
            Route::Ignore
        );
        assert_eq!(reply(session.receive(b"not json"))["error"]["code"], -32700);
        assert_eq!(reply(session.receive(&[0xff]))["error"]["code"], -32700);
        assert!(session.pending.is_empty());
        assert_eq!(
            reply(send(&mut session, request(json!(1), "unknown", json!({}))))["error"]["code"],
            -32601
        );
    }

    #[test]
    fn failed_initialization_can_be_corrected_without_early_authority() {
        let mut session = Session::default();
        send(
            &mut session,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        );
        let mut bad = init("2025-11-25");
        bad["params"]["clientInfo"] = Value::Null;
        assert_eq!(reply(send(&mut session, bad))["error"]["code"], -32602);
        assert!(reply(send(&mut session, init("2025-11-25")))["result"].is_object());
    }

    #[test]
    fn results_keep_protocol_errors_distinct_from_tool_failures() {
        let mut session = ready();
        let (id, seq, operation) = ticket(send(
            &mut session,
            request(json!(1), "tools/list", json!({})),
        ));
        assert_eq!(operation, Operation::List);
        let tools = json!([{"name":"team_inbox","inputSchema":{"type":"object"}}]);
        assert_eq!(
            session.complete(&id, seq, Ok(tools.clone())).unwrap()["result"]["tools"],
            tools
        );
        assert!(session.complete(&id, seq, Ok(json!([]))).is_none());
        for (error, expected_protocol) in [
            (BackendError::UnknownTool, true),
            (BackendError::Failed("Denied".into()), false),
        ] {
            let (id, seq, operation) = ticket(send(
                &mut session,
                request(
                    json!(2),
                    "tools/call",
                    json!({"name":"team_inbox","arguments":{"me":"untrusted"}}),
                ),
            ));
            assert_eq!(
                operation,
                Operation::Call {
                    name: "team_inbox".into(),
                    arguments: json!({"me":"untrusted"})
                }
            );
            let result = session.complete(&id, seq, Err(error)).unwrap();
            if expected_protocol {
                assert_eq!(result["error"]["code"], -32602);
            } else {
                assert_eq!(result["result"]["isError"], true);
            }
        }
        let (id, seq, _) = ticket(send(
            &mut session,
            request(json!(3), "tools/call", json!({"name":"team_inbox"})),
        ));
        let result = session
            .complete(&id, seq, Ok(json!({"messages":[]})))
            .unwrap();
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(
            serde_json::from_str::<Value>(result["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap(),
            json!({"messages":[]})
        );
    }

    #[test]
    fn cancellation_and_reused_json_ids_cannot_deliver_stale_completions() {
        let mut session = ready();
        let (id, old, _) = ticket(send(
            &mut session,
            request(json!(7), "tools/list", json!({})),
        ));
        assert_eq!(
            reply(send(
                &mut session,
                request(json!(7), "tools/list", json!({}))
            ))["error"]["code"],
            -32600
        );
        assert_eq!(
            send(
                &mut session,
                json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}})
            ),
            Route::Cancel {
                id: id.clone(),
                ticket: old
            }
        );
        let (_, new, _) = ticket(send(
            &mut session,
            request(id.clone(), "tools/list", json!({})),
        ));
        assert_ne!(old, new);
        assert!(session.complete(&id, old, Ok(json!([]))).is_none());
        assert!(session.complete(&id, new, Ok(json!([]))).unwrap()["result"].is_object());
        let (_, numeric, _) = ticket(send(
            &mut session,
            request(json!(8), "tools/list", json!({})),
        ));
        let (_, string, _) = ticket(send(
            &mut session,
            request(json!("8"), "tools/list", json!({})),
        ));
        assert!(
            session
                .complete(&json!(8), numeric, Ok(json!([])))
                .is_some()
        );
        assert!(
            session
                .complete(&json!("8"), string, Ok(json!([])))
                .is_some()
        );
    }

    #[test]
    fn saturation_preserves_pending_requests_and_sequence_never_wraps() {
        let mut session = ready();
        let mut pending = Vec::new();
        for id in 0..MAX_PENDING {
            pending.push(ticket(send(
                &mut session,
                request(json!(id), "tools/list", json!({})),
            )));
        }
        assert!(
            reply(send(
                &mut session,
                request(json!(MAX_PENDING), "tools/list", json!({}))
            ))["error"]
                .is_object()
        );
        for (id, seq, _) in pending {
            assert!(session.complete(&id, seq, Ok(json!([]))).is_some());
        }
        session.next_ticket = u64::MAX;
        assert!(
            reply(send(
                &mut session,
                request(json!(99), "tools/list", json!({}))
            ))["error"]
                .is_object()
        );
        assert!(session.pending.is_empty());
    }

    #[test]
    fn oversized_tool_results_and_catalogues_finish_with_bounded_uncertain_outcome_errors() {
        let mut session = ready();
        for value in [
            json!("x".repeat(MAX_FRAME_BYTES)),
            json!("\\".repeat(MAX_FRAME_BYTES / 3)),
        ] {
            let (id, seq, _) = ticket(send(
                &mut session,
                request(json!(1), "tools/call", json!({"name":"synthetic"})),
            ));
            let response = session.complete(&id, seq, Ok(value)).unwrap();
            assert_eq!(response["error"]["code"], -32000);
            assert!(
                response["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("may already have completed")
            );
            let mut output = Vec::new();
            write_message(&mut output, &response).unwrap();
            assert!(output.len() < 512);
            assert!(session.complete(&id, seq, Ok(json!({}))).is_none());
        }
        let (id, seq, _) = ticket(send(
            &mut session,
            request(json!(2), "tools/list", json!({})),
        ));
        let response = session
            .complete(&id, seq, Ok(json!([{"name":"x".repeat(MAX_FRAME_BYTES)}])))
            .unwrap();
        assert_eq!(response["error"]["code"], -32000);
        assert!(encode_json(&response).is_ok());
        let (id, seq, _) = ticket(send(
            &mut session,
            request(json!(3), "tools/call", json!({"name":"synthetic"})),
        ));
        let response = session
            .complete(
                &id,
                seq,
                Err(BackendError::Failed("x".repeat(MAX_FRAME_BYTES))),
            )
            .unwrap();
        assert_eq!(response["error"]["code"], -32000);
        assert!(encode_json(&response).is_ok());
        assert!(session.pending.is_empty());
    }
}
