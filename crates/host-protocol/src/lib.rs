//! Transport-independent framing and explicit remote-host capability negotiation.
//! No filesystem, process, network or credential authority is granted by this crate:
//! it decides what a frame is, whether it is allowed now, and keeps the bounded
//! bookkeeping (requests in flight, stream sequences, deduplication) a host or a
//! client needs. See `PROTOCOL.md` next to this crate for the wire contract.
mod bytes;
mod connection;
mod dedup;
mod frame;
mod ids;
mod stream;
mod wire;

pub use bytes::{
    Chunk, MAX_CHUNK_BYTES, MAX_COMPONENT_BYTES, MAX_PATH_BYTES, MAX_PATH_COMPONENTS, RelPath,
};
pub use connection::{Connection, Dispatch, Fatal, HostLimits, InFlight, Step};
pub use dedup::{Dedup, DedupCache};
pub use frame::{
    Frame, MAX_FRAME_BYTES, MAX_JSON_DEPTH, decode_payload, encode_frame, frame_length, read_frame,
    wire_error, write_frame,
};
pub use ids::{DedupKey, Epoch, MAX_ID, StreamId};
pub use stream::{Ack, AckError, Arrival, SendError, StreamPos, StreamReceiver, StreamSender};
pub use wire::{
    Budgets, Cancel, Capability, ClientFrame, ClientMessage, ErrorCode, Hello, HelloOk, HostFrame,
    HostInfo, Persistence, PersistenceMethod, RawMessage, ReplyMessage, Survival, VERSION,
    WireError,
};

/// Kept for code written against the foundation: the largest request identifier.
pub const MAX_REQUEST_ID: u64 = MAX_ID;
