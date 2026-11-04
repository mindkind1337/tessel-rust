use crate::ids::{Epoch, MAX_ID};
use crate::stream::{Ack, StreamPos, valid_seq};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const VERSION: u16 = 1;
const MAX_KIND_BYTES: usize = 64;
const MAX_DETAIL_BYTES: usize = 200;
const MAX_TOTAL_RING_BYTES: u32 = 64 * 1024 * 1024;
const MAX_STREAM_WINDOW_BYTES: u32 = 16 * 1024 * 1024;
const MAX_IN_FLIGHT: u16 = 256;

/// What a host can do, negotiated one by one: a client may take tree and git while keeping
/// its terminals elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Pty,
    Tree,
    Git,
    Editor,
    #[serde(rename = "tunnel.reverse")]
    TunnelReverse,
}

impl Capability {
    pub const ALL: [Capability; 5] = [
        Self::Pty,
        Self::Tree,
        Self::Git,
        Self::Editor,
        Self::TunnelReverse,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Pty => "pty",
            Self::Tree => "tree",
            Self::Git => "git",
            Self::Editor => "editor",
            Self::TunnelReverse => "tunnel.reverse",
        }
    }

    /// The capability a request kind belongs to. None: a core message, or a kind no
    /// capability of this version knows.
    pub fn for_kind(kind: &str) -> Option<Self> {
        let family = kind.split('.').next().unwrap_or(kind);
        match family {
            "terminal" => Some(Self::Pty),
            "tree" | "root" => Some(Self::Tree),
            "file" => Some(Self::Editor),
            "git" => Some(Self::Git),
            "tunnel" => Some(Self::TunnelReverse),
            _ => None,
        }
    }
}

/// Resource limits, offered by each side; the smaller of each pair applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub frame_bytes: u32,
    pub terminal_ring_bytes: u32,
    pub total_ring_bytes: u32,
    /// Unacknowledged event bytes one stream may have in flight.
    pub stream_window_bytes: u32,
    /// Requests one connection may have waiting for their reply.
    pub in_flight: u16,
}

impl Budgets {
    /// The default for a slow or shared host.
    pub const SLOW_HOST: Self = Self {
        frame_bytes: crate::frame::MAX_FRAME_BYTES as u32,
        terminal_ring_bytes: 2 * 1024 * 1024,
        total_ring_bytes: 16 * 1024 * 1024,
        stream_window_bytes: 2 * 1024 * 1024,
        in_flight: 32,
    };

    pub fn validate(self) -> Result<(), WireError> {
        let valid = (1024..=crate::frame::MAX_FRAME_BYTES as u32).contains(&self.frame_bytes)
            && self.terminal_ring_bytes > 0
            && self.terminal_ring_bytes <= self.total_ring_bytes
            && self.total_ring_bytes <= MAX_TOTAL_RING_BYTES
            && (self.frame_bytes..=MAX_STREAM_WINDOW_BYTES).contains(&self.stream_window_bytes)
            && (1..=MAX_IN_FLIGHT).contains(&self.in_flight);
        if valid {
            Ok(())
        } else {
            Err(WireError::detail(
                ErrorCode::BadRequest,
                "invalid resource budgets",
            ))
        }
    }

    /// Each limit only goes down. Two valid budgets give a valid one.
    pub fn intersect(self, peer: Self) -> Self {
        Self {
            frame_bytes: self.frame_bytes.min(peer.frame_bytes),
            terminal_ring_bytes: self.terminal_ring_bytes.min(peer.terminal_ring_bytes),
            total_ring_bytes: self.total_ring_bytes.min(peer.total_ring_bytes),
            stream_window_bytes: self.stream_window_bytes.min(peer.stream_window_bytes),
            in_flight: self.in_flight.min(peer.in_flight),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub versions: Vec<u16>,
    // Strings, so a newer client can ask for an optional capability an older host ignores.
    pub capabilities: Vec<String>,
    pub budgets: Budgets,
}

impl Hello {
    pub fn validate(&self) -> Result<(), WireError> {
        self.budgets.validate()?;
        let names_ok = self.capabilities.iter().all(|c| {
            !c.is_empty()
                && c.len() <= MAX_KIND_BYTES
                && c.bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_')
        });
        if self.versions.is_empty()
            || self.versions.len() > 8
            || self.capabilities.len() > 32
            || !names_ok
        {
            return Err(WireError::detail(
                ErrorCode::BadRequest,
                "invalid hello request",
            ));
        }
        Ok(())
    }

    /// The host's answer: the highest version both speak, the capabilities both want (in the
    /// host's order, once each), and the smaller budgets.
    pub fn negotiate(
        &self,
        supported: &[Capability],
        limits: Budgets,
        epoch: &Epoch,
    ) -> Result<HelloOk, WireError> {
        self.validate()?;
        limits.validate()?;
        if !self.versions.contains(&VERSION) {
            return Err(WireError::new(ErrorCode::UnsupportedVersion));
        }
        let mut capabilities = Vec::new();
        for capability in supported {
            if self.capabilities.iter().any(|c| c == capability.name())
                && !capabilities.contains(capability)
            {
                capabilities.push(*capability);
            }
        }
        Ok(HelloOk {
            version: VERSION,
            capabilities,
            budgets: self.budgets.intersect(limits),
            host_epoch: epoch.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloOk {
    pub version: u16,
    pub capabilities: Vec<Capability>,
    pub budgets: Budgets,
    pub host_epoch: Epoch,
}

impl HelloOk {
    pub fn validate(&self) -> Result<(), WireError> {
        self.budgets.validate()?;
        let unique = self
            .capabilities
            .iter()
            .enumerate()
            .all(|(i, c)| !self.capabilities[..i].contains(c));
        if self.version != VERSION || self.capabilities.len() > Capability::ALL.len() || !unique {
            return Err(WireError::detail(
                ErrorCode::BadRequest,
                "invalid hello response",
            ));
        }
        Ok(())
    }
}

/// Stop the request `target`, which then gets a `cancelled` error as its one reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    pub target: u64,
}

/// A message as it travels: a kind, and a body checked by whoever handles that kind. A
/// message of a kind the receiver does not know is refused with an error, not a dropped
/// connection, so a newer peer can be told plainly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMessage {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub body: Value,
}

impl RawMessage {
    pub fn new(kind: &str, body: Value) -> Self {
        Self {
            kind: kind.to_owned(),
            body,
        }
    }

    pub fn empty(kind: &str) -> Self {
        Self::new(kind, Value::Null)
    }

    pub fn of<T: Serialize>(kind: &str, body: &T) -> Self {
        Self::new(kind, serde_json::to_value(body).unwrap_or(Value::Null))
    }

    /// The body as `T`. A refusal never repeats what was sent.
    pub fn body_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, WireError> {
        T::deserialize(&self.body)
            .map_err(|_| WireError::detail(ErrorCode::BadRequest, "invalid message body"))
    }

    fn require_empty(&self) -> Result<(), WireError> {
        if self.body.is_null() {
            Ok(())
        } else {
            Err(WireError::detail(
                ErrorCode::BadRequest,
                "this message has no body",
            ))
        }
    }

    fn validate(&self) -> Result<(), WireError> {
        let kind_ok = !self.kind.is_empty()
            && self.kind.len() <= MAX_KIND_BYTES
            && self.kind.as_bytes()[0].is_ascii_lowercase()
            && self
                .kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_');
        if !kind_ok {
            return Err(WireError::detail(
                ErrorCode::BadFrame,
                "invalid message kind",
            ));
        }
        if !plain_json(&self.body, 0) {
            return Err(WireError::detail(
                ErrorCode::BadFrame,
                "numbers must be integers a JavaScript number holds exactly",
            ));
        }
        Ok(())
    }
}

/// The protocol has no fractions and no integer beyond ±(2^53 − 1): every number means the
/// same on both sides, a JavaScript client included, and survives being sent again.
fn plain_json(value: &Value, depth: usize) -> bool {
    if depth > crate::frame::MAX_JSON_DEPTH {
        return false;
    }
    match value {
        Value::Number(n) => n
            .as_u64()
            .map(|u| u <= MAX_ID)
            .or_else(|| n.as_i64().map(|i| i.unsigned_abs() <= MAX_ID))
            .unwrap_or(false),
        Value::Array(items) => items.iter().all(|v| plain_json(v, depth + 1)),
        Value::Object(map) => map.values().all(|v| plain_json(v, depth + 1)),
        _ => true,
    }
}

/// The messages every connection understands, whatever was negotiated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    Hello(Hello),
    Ping,
    Cancel(Cancel),
}

impl ClientMessage {
    pub fn into_raw(self) -> RawMessage {
        match self {
            Self::Hello(hello) => RawMessage::of("hello", &hello),
            Self::Ping => RawMessage::empty("ping"),
            Self::Cancel(cancel) => RawMessage::of("cancel", &cancel),
        }
    }

    /// None: not a core message (it belongs to a capability, or is unknown).
    pub fn from_raw(message: &RawMessage) -> Result<Option<Self>, WireError> {
        Ok(Some(match message.kind.as_str() {
            "hello" => Self::Hello(message.body_as()?),
            "ping" => {
                message.require_empty()?;
                Self::Ping
            }
            "cancel" => Self::Cancel(message.body_as()?),
            _ => return Ok(None),
        }))
    }
}

/// The successful replies to core messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyMessage {
    HelloOk(HelloOk),
    Pong,
    /// A request that has nothing more to say than that it was done.
    Done,
}

impl ReplyMessage {
    pub fn into_raw(self) -> RawMessage {
        match self {
            Self::HelloOk(ok) => RawMessage::of("hello_ok", &ok),
            Self::Pong => RawMessage::empty("pong"),
            Self::Done => RawMessage::empty("done"),
        }
    }

    pub fn from_raw(message: &RawMessage) -> Result<Option<Self>, WireError> {
        Ok(Some(match message.kind.as_str() {
            "hello_ok" => {
                let ok: HelloOk = message.body_as()?;
                ok.validate()?;
                Self::HelloOk(ok)
            }
            "pong" => {
                message.require_empty()?;
                Self::Pong
            }
            "done" => {
                message.require_empty()?;
                Self::Done
            }
            _ => return Ok(None),
        }))
    }
}

/// Why something was refused. The detail is a fixed sentence of the host's, never a copy of
/// what the client sent, so a refusal can be logged without leaking a command or a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The frame itself is broken: the connection closes.
    BadFrame,
    BadRequest,
    UnsupportedVersion,
    UnsupportedKind,
    /// Nothing but hello is allowed before hello.
    NotNegotiated,
    CapabilityNotNegotiated,
    /// Too many requests or entries at once: try again later.
    Busy,
    Cancelled,
    NotFound,
    PermissionDenied,
    /// Someone else changed it first (a save over a newer file).
    Conflict,
    TooLarge,
    /// The history needed to resume is gone: take a snapshot or start over.
    Gap,
    /// The host restarted since: what the client knew is gone.
    EpochMismatch,
    Timeout,
    Internal,
    /// A code from a newer peer.
    #[serde(other)]
    Unknown,
}

impl ErrorCode {
    pub fn retryable(self) -> bool {
        matches!(self, Self::Busy | Self::Timeout)
    }

    fn sentence(self) -> &'static str {
        match self {
            Self::BadFrame => "malformed frame",
            Self::BadRequest => "invalid request",
            Self::UnsupportedVersion => "unsupported protocol version",
            Self::UnsupportedKind => "unsupported message kind",
            Self::NotNegotiated => "hello is required first",
            Self::CapabilityNotNegotiated => "capability was not negotiated",
            Self::Busy => "too many requests in flight",
            Self::Cancelled => "cancelled",
            Self::NotFound => "not found",
            Self::PermissionDenied => "permission denied",
            Self::Conflict => "changed by someone else",
            Self::TooLarge => "too large",
            Self::Gap => "history no longer available",
            Self::EpochMismatch => "host restarted",
            Self::Timeout => "timed out",
            Self::Internal => "internal error",
            Self::Unknown => "unknown error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireError {
    pub code: ErrorCode,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl WireError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            retryable: code.retryable(),
            detail: None,
        }
    }

    pub fn detail(code: ErrorCode, detail: &'static str) -> Self {
        Self {
            detail: Some(detail.to_owned()),
            ..Self::new(code)
        }
    }

    fn validate(&self) -> Result<(), WireError> {
        match &self.detail {
            Some(d)
                if d.len() > MAX_DETAIL_BYTES || !d.bytes().all(|b| (0x20..0x7f).contains(&b)) =>
            {
                Err(WireError::detail(
                    ErrorCode::BadFrame,
                    "invalid error detail",
                ))
            }
            _ => Ok(()),
        }
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.code.sentence()),
            None => f.write_str(self.code.sentence()),
        }
    }
}

impl std::error::Error for WireError {}

fn check_version(version: u16) -> Result<(), WireError> {
    if version == VERSION {
        Ok(())
    } else {
        Err(WireError::new(ErrorCode::UnsupportedVersion))
    }
}

fn shape_error() -> WireError {
    WireError::detail(ErrorCode::BadFrame, "invalid frame shape")
}

/// What a client sends: a request (`request_id` + `message`), or an acknowledgment (`ack`),
/// which gets no reply. Request identifiers go up within a connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientFrame {
    pub version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<RawMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack: Option<Ack>,
}

impl ClientFrame {
    pub fn request(request_id: u64, message: ClientMessage) -> Self {
        Self::request_raw(request_id, message.into_raw())
    }

    pub fn request_raw(request_id: u64, message: RawMessage) -> Self {
        Self {
            version: VERSION,
            request_id: Some(request_id),
            message: Some(message),
            ack: None,
        }
    }

    pub fn ack(ack: Ack) -> Self {
        Self {
            version: VERSION,
            request_id: None,
            message: None,
            ack: Some(ack),
        }
    }

    pub fn check(&self) -> Result<(), WireError> {
        check_version(self.version)?;
        match (self.request_id, &self.message, &self.ack) {
            (Some(id), Some(message), None) if (1..=MAX_ID).contains(&id) => message.validate(),
            (None, None, Some(ack)) if valid_seq(ack.seq) => Ok(()),
            _ => Err(shape_error()),
        }
    }
}

/// What a host sends: the one reply to a request (`reply_to` + `ok` or `error`), an event of a
/// stream (`stream` + `event`), or an error about the whole connection (`error` alone), after
/// which the host closes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostFrame {
    pub version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<RawMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamPos>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<RawMessage>,
}

impl HostFrame {
    fn empty() -> Self {
        Self {
            version: VERSION,
            reply_to: None,
            ok: None,
            error: None,
            stream: None,
            event: None,
        }
    }

    pub fn reply_ok(request_id: u64, message: RawMessage) -> Self {
        Self {
            reply_to: Some(request_id),
            ok: Some(message),
            ..Self::empty()
        }
    }

    pub fn reply_error(request_id: u64, error: WireError) -> Self {
        Self {
            reply_to: Some(request_id),
            error: Some(error),
            ..Self::empty()
        }
    }

    pub fn connection_error(error: WireError) -> Self {
        Self {
            error: Some(error),
            ..Self::empty()
        }
    }

    pub fn event(stream: StreamPos, message: RawMessage) -> Self {
        Self {
            stream: Some(stream),
            event: Some(message),
            ..Self::empty()
        }
    }

    pub fn check(&self) -> Result<(), WireError> {
        check_version(self.version)?;
        let reply_ok = |id: u64| (1..=MAX_ID).contains(&id);
        match (
            self.reply_to,
            &self.ok,
            &self.error,
            &self.stream,
            &self.event,
        ) {
            (Some(id), Some(message), None, None, None) if reply_ok(id) => message.validate(),
            (Some(id), None, Some(error), None, None) if reply_ok(id) => error.validate(),
            (None, None, Some(error), None, None) => error.validate(),
            (None, None, None, Some(stream), Some(event)) if valid_seq(stream.seq) => {
                event.validate()
            }
            _ => Err(shape_error()),
        }
    }
}
