//! Transport-independent framing and explicit remote-host capability negotiation.
//! No filesystem, process, network or credential authority is granted by this crate.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_REQUEST_ID: u64 = (1u64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Pty,
    Tree,
    Git,
    Editor,
    #[serde(rename = "tunnel.reverse")]
    TunnelReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub frame_bytes: u32,
    pub terminal_ring_bytes: u32,
    pub total_ring_bytes: u32,
}

impl Budgets {
    pub const SLOW_HOST: Self = Self {
        frame_bytes: MAX_FRAME_BYTES as u32,
        terminal_ring_bytes: 2 * 1024 * 1024,
        total_ring_bytes: 16 * 1024 * 1024,
    };

    pub fn validate(self) -> io::Result<()> {
        if !(1024..=MAX_FRAME_BYTES as u32).contains(&self.frame_bytes)
            || self.terminal_ring_bytes == 0
            || self.terminal_ring_bytes > self.total_ring_bytes
            || self.total_ring_bytes > 64 * 1024 * 1024
        {
            return Err(invalid("invalid resource budgets"));
        }
        Ok(())
    }

    fn intersect(self, peer: Self) -> Self {
        Self {
            frame_bytes: self.frame_bytes.min(peer.frame_bytes),
            terminal_ring_bytes: self.terminal_ring_bytes.min(peer.terminal_ring_bytes),
            total_ring_bytes: self.total_ring_bytes.min(peer.total_ring_bytes),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub versions: Vec<u16>,
    // Strings permit a newer client to request optional capabilities an older host ignores.
    pub capabilities: Vec<String>,
    pub budgets: Budgets,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloOk {
    pub version: u16,
    pub capabilities: Vec<Capability>,
    pub budgets: Budgets,
    pub host_epoch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Message {
    Hello(Hello),
    HelloOk(HelloOk),
    Ping,
    Pong,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u16,
    pub request_id: u64,
    pub message: Message,
}

/// Connection-local negotiation state. A bridge must create one per connection;
/// this deliberately grants no process/file operations before negotiation.
pub struct Handshake {
    negotiated: Option<HelloOk>,
    last_request_id: u64,
}

impl Default for Handshake {
    fn default() -> Self {
        Self::new()
    }
}

impl Handshake {
    pub fn new() -> Self {
        Self {
            negotiated: None,
            last_request_id: 0,
        }
    }

    pub fn negotiated(&self) -> Option<&HelloOk> {
        self.negotiated.as_ref()
    }

    pub fn accept(
        &mut self,
        request: Envelope,
        supported: &[Capability],
        limits: Budgets,
        epoch: &str,
    ) -> io::Result<Envelope> {
        request.validate()?;
        if request.request_id <= self.last_request_id {
            return Err(invalid("request identifier must increase"));
        }
        let message = match (&self.negotiated, request.message) {
            (None, Message::Hello(hello)) => {
                let agreed = hello.negotiate(supported, limits, epoch)?;
                self.negotiated = Some(agreed.clone());
                Message::HelloOk(agreed)
            }
            (Some(_), Message::Ping) => Message::Pong,
            _ => return Err(invalid("message is not valid in this connection state")),
        };
        self.last_request_id = request.request_id;
        Ok(Envelope {
            version: VERSION,
            request_id: request.request_id,
            message,
        })
    }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

impl Envelope {
    pub fn validate(&self) -> io::Result<()> {
        if self.version != VERSION || !(1..=MAX_REQUEST_ID).contains(&self.request_id) {
            return Err(invalid("unsupported version or invalid request identifier"));
        }
        match &self.message {
            Message::Hello(hello) => hello.validate(),
            Message::HelloOk(hello) => {
                hello.budgets.validate()?;
                if hello.version != VERSION
                    || !valid_epoch(&hello.host_epoch)
                    || hello.capabilities.len() > 5
                    || hello
                        .capabilities
                        .iter()
                        .enumerate()
                        .any(|(i, c)| hello.capabilities[..i].contains(c))
                {
                    return Err(invalid("invalid hello response"));
                }
                Ok(())
            }
            Message::Ping | Message::Pong => Ok(()),
        }
    }
}

fn valid_epoch(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Hello {
    pub fn validate(&self) -> io::Result<()> {
        self.budgets.validate()?;
        if self.versions.is_empty()
            || self.versions.len() > 8
            || self.capabilities.len() > 32
            || self.capabilities.iter().any(|c| {
                c.is_empty()
                    || c.len() > 64
                    || !c
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_')
            })
        {
            return Err(invalid("invalid hello request"));
        }
        Ok(())
    }

    pub fn negotiate(
        &self,
        supported: &[Capability],
        limits: Budgets,
        epoch: &str,
    ) -> io::Result<HelloOk> {
        self.validate()?;
        limits.validate()?;
        if !self.versions.contains(&VERSION) || !valid_epoch(epoch) {
            return Err(invalid("no supported version or invalid host epoch"));
        }
        let mut capabilities = Vec::new();
        for capability in supported {
            let name = match capability {
                Capability::Pty => "pty",
                Capability::Tree => "tree",
                Capability::Git => "git",
                Capability::Editor => "editor",
                Capability::TunnelReverse => "tunnel.reverse",
            };
            if self.capabilities.iter().any(|c| c == name) && !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
        Ok(HelloOk {
            version: VERSION,
            capabilities,
            budgets: self.budgets.intersect(limits),
            host_epoch: epoch.to_owned(),
        })
    }
}

/// Clean EOF between frames returns None. Partial headers/bodies fail closed.
/// The caller must impose transport deadlines and discard the connection after any error.
pub fn read_frame(reader: &mut impl Read, limit: usize) -> io::Result<Option<Envelope>> {
    let mut header = [0u8; 4];
    loop {
        match reader.read(&mut header[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    reader.read_exact(&mut header[1..])?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > limit.min(MAX_FRAME_BYTES) {
        return Err(invalid("frame length exceeds limit"));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    let envelope: Envelope =
        serde_json::from_slice(&payload).map_err(|_| invalid("invalid protocol JSON"))?;
    envelope.validate()?;
    Ok(Some(envelope))
}

/// Validate and serialize within the same bound before writing any bytes.
pub fn write_frame(writer: &mut impl Write, message: &Envelope, limit: usize) -> io::Result<()> {
    message.validate()?;
    let mut buffer = LimitedBuffer {
        bytes: Vec::new(),
        limit: limit.min(MAX_FRAME_BYTES),
    };
    serde_json::to_writer(&mut buffer, message)
        .map_err(|_| invalid("outgoing frame exceeds limit"))?;
    writer.write_all(&(buffer.bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&buffer.bytes)
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(invalid("frame limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
