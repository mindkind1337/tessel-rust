use crate::wire::{ClientFrame, ErrorCode, HostFrame, WireError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{self, Read, Write};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Objects and arrays inside one frame, counted before parsing.
pub const MAX_JSON_DEPTH: usize = 16;

/// A frame that can travel: it checks its own shape once decoded and before encoding.
pub trait Frame: Serialize + DeserializeOwned {
    fn check(&self) -> Result<(), WireError>;
}

impl Frame for ClientFrame {
    fn check(&self) -> Result<(), WireError> {
        ClientFrame::check(self)
    }
}

impl Frame for HostFrame {
    fn check(&self) -> Result<(), WireError> {
        HostFrame::check(self)
    }
}

fn refused(error: WireError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn bad_frame(detail: &'static str) -> io::Error {
    refused(WireError::detail(ErrorCode::BadFrame, detail))
}

/// The protocol error inside an error from this module, to tell the peer why.
pub fn wire_error(error: &io::Error) -> Option<&WireError> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<WireError>())
}

/// The payload length a header announces, refused above `limit` before anything is read.
pub fn frame_length(header: [u8; 4], limit: usize) -> io::Result<usize> {
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > limit.min(MAX_FRAME_BYTES) {
        return Err(bad_frame("frame length exceeds limit"));
    }
    Ok(length)
}

fn depth_within(bytes: &[u8], max: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > max {
                    return false;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

/// One payload, decoded and checked. Errors never contain what was sent.
pub fn decode_payload<T: Frame>(payload: &[u8]) -> io::Result<T> {
    if !depth_within(payload, MAX_JSON_DEPTH) {
        return Err(bad_frame("JSON nested too deeply"));
    }
    let frame: T =
        serde_json::from_slice(payload).map_err(|_| bad_frame("invalid protocol JSON"))?;
    frame.check().map_err(refused)?;
    Ok(frame)
}

/// Header and payload in one buffer, checked and bounded before any byte exists.
pub fn encode_frame<T: Frame>(frame: &T, limit: usize) -> io::Result<Vec<u8>> {
    frame.check().map_err(refused)?;
    let mut buffer = LimitedBuffer {
        bytes: vec![0; 4],
        limit: 4 + limit.min(MAX_FRAME_BYTES),
    };
    serde_json::to_writer(&mut buffer, frame).map_err(|_| {
        refused(WireError::detail(
            ErrorCode::TooLarge,
            "outgoing frame exceeds limit",
        ))
    })?;
    let length = (buffer.bytes.len() - 4) as u32;
    buffer.bytes[..4].copy_from_slice(&length.to_be_bytes());
    Ok(buffer.bytes)
}

/// Clean EOF between frames returns None. Partial headers/bodies fail closed.
/// The caller must impose transport deadlines and discard the connection after any error.
pub fn read_frame<T: Frame>(reader: &mut impl Read, limit: usize) -> io::Result<Option<T>> {
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
    let length = frame_length(header, limit)?;
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    decode_payload(&payload).map(Some)
}

/// Validate and serialize within the same bound before writing any bytes.
pub fn write_frame<T: Frame>(writer: &mut impl Write, frame: &T, limit: usize) -> io::Result<()> {
    let bytes = encode_frame(frame, limit)?;
    writer.write_all(&bytes)
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("frame limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
