use crate::ids::Epoch;
use crate::stream::Ack;
use crate::wire::{
    Budgets, Capability, ClientFrame, ClientMessage, ErrorCode, HelloOk, HostFrame, HostInfo,
    RawMessage, ReplyMessage, WireError,
};
use std::collections::HashMap;

/// What this host offers to every connection.
#[derive(Clone, Debug)]
pub struct HostLimits {
    pub capabilities: Vec<Capability>,
    pub budgets: Budgets,
    pub epoch: Epoch,
    pub info: HostInfo,
}

/// A refusal that ends the connection: send `frame()`, then close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fatal {
    pub reply_to: Option<u64>,
    pub error: WireError,
}

impl Fatal {
    fn new(reply_to: Option<u64>, error: WireError) -> Self {
        Self { reply_to, error }
    }

    pub fn frame(&self) -> HostFrame {
        match self.reply_to {
            Some(id) => HostFrame::reply_error(id, self.error.clone()),
            None => HostFrame::connection_error(self.error.clone()),
        }
    }
}

/// A request for a capability, admitted: its handler answers it with `Connection::finish`.
#[derive(Clone, Debug, PartialEq)]
pub struct Dispatch {
    pub request_id: u64,
    pub capability: Capability,
    pub message: RawMessage,
}

/// What to do with one frame from the client.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Send this, now; the request is answered.
    Reply(HostFrame),
    /// Hand it to the capability's handler.
    Dispatch(Dispatch),
    /// An acknowledgment, for the stream it names.
    Ack(Ack),
}

/// Requests waiting for their one reply, at most `capacity` of them.
#[derive(Clone, Debug)]
pub struct InFlight {
    capacity: usize,
    cancelled: HashMap<u64, bool>,
}

impl InFlight {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            cancelled: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.cancelled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cancelled.is_empty()
    }

    pub fn begin(&mut self, id: u64) -> Result<(), WireError> {
        if self.cancelled.len() >= self.capacity {
            return Err(WireError::new(ErrorCode::Busy));
        }
        self.cancelled.insert(id, false);
        Ok(())
    }

    /// Whether `id` was waiting and is now marked cancelled (once).
    pub fn cancel(&mut self, id: u64) -> bool {
        match self.cancelled.get_mut(&id) {
            Some(flag) if !*flag => {
                *flag = true;
                true
            }
            _ => false,
        }
    }

    pub fn is_cancelled(&self, id: u64) -> bool {
        self.cancelled.get(&id).copied().unwrap_or(false)
    }

    /// Removes `id`: Some(cancelled) the first time, None after (so a request never gets
    /// two replies).
    pub fn finish(&mut self, id: u64) -> Option<bool> {
        self.cancelled.remove(&id)
    }
}

/// One client connection, host side: hello first, identifiers going up, only negotiated
/// capabilities, a bounded number of requests in flight, one reply per request.
pub struct Connection {
    limits: HostLimits,
    negotiated: Option<HelloOk>,
    last_request_id: u64,
    in_flight: InFlight,
}

impl Connection {
    pub fn new(limits: HostLimits) -> Self {
        Self {
            limits,
            negotiated: None,
            last_request_id: 0,
            in_flight: InFlight::new(0),
        }
    }

    pub fn negotiated(&self) -> Option<&HelloOk> {
        self.negotiated.as_ref()
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub fn is_cancelled(&self, request_id: u64) -> bool {
        self.in_flight.is_cancelled(request_id)
    }

    pub fn on_frame(&mut self, frame: ClientFrame) -> Result<Step, Fatal> {
        frame.check().map_err(|error| Fatal::new(None, error))?;
        if let Some(ack) = frame.ack {
            if self.negotiated.is_none() {
                return Err(Fatal::new(None, WireError::new(ErrorCode::NotNegotiated)));
            }
            return Ok(Step::Ack(ack));
        }
        let (Some(id), Some(message)) = (frame.request_id, frame.message) else {
            return Err(Fatal::new(
                None,
                WireError::detail(ErrorCode::BadFrame, "invalid frame shape"),
            ));
        };
        if id <= self.last_request_id {
            return Err(Fatal::new(
                None,
                WireError::detail(ErrorCode::BadFrame, "request identifier must increase"),
            ));
        }
        self.last_request_id = id;
        let reply = |result: Result<ReplyMessage, WireError>| {
            Ok(Step::Reply(match result {
                Ok(message) => HostFrame::reply_ok(id, message.into_raw()),
                Err(error) => HostFrame::reply_error(id, error),
            }))
        };
        let core = ClientMessage::from_raw(&message);
        let Some(negotiated) = &self.negotiated else {
            // Before hello, anything else ends the connection: nothing is allowed yet.
            return match core {
                Ok(Some(ClientMessage::Hello(hello))) => {
                    let agreed = hello
                        .negotiate(
                            &self.limits.capabilities,
                            self.limits.budgets,
                            &self.limits.epoch,
                        )
                        .map_err(|error| Fatal::new(Some(id), error))?;
                    self.in_flight = InFlight::new(usize::from(agreed.budgets.in_flight));
                    self.negotiated = Some(agreed.clone());
                    reply(Ok(ReplyMessage::HelloOk(agreed)))
                }
                Err(error) if message.kind == "hello" => Err(Fatal::new(Some(id), error)),
                _ => Err(Fatal::new(
                    Some(id),
                    WireError::new(ErrorCode::NotNegotiated),
                )),
            };
        };
        match core {
            Err(error) => reply(Err(error)),
            Ok(Some(ClientMessage::Hello(_))) => reply(Err(WireError::detail(
                ErrorCode::BadRequest,
                "hello was already accepted",
            ))),
            Ok(Some(ClientMessage::Ping)) => reply(Ok(ReplyMessage::Pong)),
            Ok(Some(ClientMessage::HostInfo)) => {
                reply(Ok(ReplyMessage::HostInfo(self.limits.info.clone())))
            }
            Ok(Some(ClientMessage::Cancel(cancel))) => {
                if cancel.target == 0 || cancel.target >= id {
                    reply(Err(WireError::detail(
                        ErrorCode::BadRequest,
                        "cancel targets an earlier request",
                    )))
                } else if self.in_flight.cancel(cancel.target) {
                    reply(Ok(ReplyMessage::Done))
                } else {
                    reply(Err(WireError::new(ErrorCode::NotFound)))
                }
            }
            Ok(None) => match Capability::for_kind(&message.kind) {
                None => reply(Err(WireError::new(ErrorCode::UnsupportedKind))),
                Some(capability) if !negotiated.capabilities.contains(&capability) => {
                    reply(Err(WireError::new(ErrorCode::CapabilityNotNegotiated)))
                }
                Some(capability) => match self.in_flight.begin(id) {
                    Err(error) => reply(Err(error)),
                    Ok(()) => Ok(Step::Dispatch(Dispatch {
                        request_id: id,
                        capability,
                        message,
                    })),
                },
            },
        }
    }

    /// The one reply to a dispatched request: `cancelled` if it was cancelled meanwhile,
    /// nothing if it was already answered.
    pub fn finish(
        &mut self,
        request_id: u64,
        result: Result<RawMessage, WireError>,
    ) -> Option<HostFrame> {
        let cancelled = self.in_flight.finish(request_id)?;
        Some(match (cancelled, result) {
            (true, _) => HostFrame::reply_error(request_id, WireError::new(ErrorCode::Cancelled)),
            (false, Ok(message)) => HostFrame::reply_ok(request_id, message),
            (false, Err(error)) => HostFrame::reply_error(request_id, error),
        })
    }
}
