use crate::ids::{MAX_ID, StreamId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Where an event sits in its stream: sequence numbers start at 1 and go up by exactly 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamPos {
    pub id: StreamId,
    pub seq: u64,
}

/// The client has received every event of `stream` up to and including `seq`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ack {
    pub stream: StreamId,
    pub seq: u64,
}

pub(crate) fn valid_seq(seq: u64) -> bool {
    (1..=MAX_ID).contains(&seq)
}

/// What a receiver does with an event, given its sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// The next one: apply it.
    Next,
    /// Already applied (a replay overlapping what was seen): drop it.
    Duplicate,
    /// Something is missing: never apply it; ask for a snapshot or a reset.
    Gap { expected: u64 },
}

/// The client side of one stream: applies each event once and in order.
#[derive(Clone, Debug, Default)]
pub struct StreamReceiver {
    last: u64,
}

impl StreamReceiver {
    /// After a reconnect: everything up to `last` was applied before.
    pub fn resume_after(last: u64) -> Self {
        Self { last }
    }

    pub fn last(&self) -> u64 {
        self.last
    }

    pub fn accept(&mut self, seq: u64) -> Arrival {
        if seq <= self.last {
            Arrival::Duplicate
        } else if seq == self.last + 1 {
            self.last = seq;
            Arrival::Next
        } else {
            Arrival::Gap {
                expected: self.last + 1,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The client has not acknowledged enough: wait (the producer keeps its data in its own
    /// bounded buffer, or drops replay history and marks a gap).
    WindowFull,
    /// Every sequence number was used: the stream must be reset.
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckError {
    /// An acknowledgment for an event never sent: a protocol violation.
    Unsent,
}

/// The host side of one stream: numbers events and keeps at most `window` unacknowledged
/// bytes in flight, so a slow client cannot make the host buffer without bound.
#[derive(Clone, Debug)]
pub struct StreamSender {
    window: u64,
    next: u64,
    acked: u64,
    pending: VecDeque<(u64, u64)>,
    unacked: u64,
}

impl StreamSender {
    pub fn new(window_bytes: u32) -> Self {
        Self {
            window: u64::from(window_bytes.max(1)),
            next: 1,
            acked: 0,
            pending: VecDeque::new(),
            unacked: 0,
        }
    }

    /// Takes the next sequence number for an event of `bytes`. With nothing in flight one
    /// event is always admitted, so an event larger than the window cannot stall the stream.
    pub fn reserve(&mut self, bytes: usize) -> Result<u64, SendError> {
        if self.next > MAX_ID {
            return Err(SendError::Exhausted);
        }
        let bytes = bytes as u64;
        if !self.pending.is_empty() && self.unacked.saturating_add(bytes) > self.window {
            return Err(SendError::WindowFull);
        }
        let seq = self.next;
        self.next += 1;
        self.pending.push_back((seq, bytes));
        self.unacked += bytes;
        Ok(seq)
    }

    /// An acknowledgment older than one already seen is ignored (it crossed a newer one).
    pub fn ack(&mut self, seq: u64) -> Result<(), AckError> {
        if seq >= self.next {
            return Err(AckError::Unsent);
        }
        if seq <= self.acked {
            return Ok(());
        }
        while let Some(&(sent, bytes)) = self.pending.front() {
            if sent > seq {
                break;
            }
            self.pending.pop_front();
            self.unacked -= bytes;
        }
        self.acked = seq;
        Ok(())
    }

    pub fn last_sent(&self) -> u64 {
        self.next - 1
    }

    pub fn acked(&self) -> u64 {
        self.acked
    }

    pub fn unacked_bytes(&self) -> u64 {
        self.unacked
    }
}
