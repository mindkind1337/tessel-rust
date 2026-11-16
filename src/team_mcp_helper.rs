//! Dedicated child-process stdio bridge. Never run this loop inside the GUI.
//! Blocking stdin/stdout are isolated on bounded worker queues; main exits the
//! helper process on timeout rather than joining a thread stuck in a pipe syscall.
use crate::team_mcp_broker::Client;
use crate::team_mcp_stdio::{self, BackendError, Route, Session};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

const PIPE_DEADLINE: Duration = Duration::from_secs(10);
const INITIALIZE_DEADLINE: Duration = Duration::from_secs(30);
const MAX_JOBS: usize = 4;
const MAX_OUTPUT: usize = 8;

enum Event {
    Input(Vec<u8>),
    End,
    InputFailed,
    Output(u64, bool),
    Completed {
        id: Value,
        ticket: u64,
        result: Result<Value, BackendError>,
    },
}
struct Output {
    sequence: u64,
    value: Value,
}

#[derive(Default)]
struct Framer {
    bytes: Vec<u8>,
    started: Option<Instant>,
}
impl Framer {
    fn expired(&self, now: Instant) -> bool {
        self.started
            .is_some_and(|started| now.duration_since(started) >= PIPE_DEADLINE)
    }
    fn feed(&mut self, bytes: &[u8], now: Instant) -> Result<Vec<Vec<u8>>, String> {
        if self.expired(now) {
            return Err("MCP input frame timed out".into());
        }
        let mut frames = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if part.len() > team_mcp_stdio::MAX_FRAME_BYTES.saturating_sub(self.bytes.len()) {
                return Err("MCP input frame exceeds limit".into());
            }
            self.started.get_or_insert(now);
            self.bytes.extend_from_slice(part);
            if self.bytes.last() == Some(&b'\n') {
                frames.push(std::mem::take(&mut self.bytes));
                self.started = None;
            }
        }
        Ok(frames)
    }
}

#[derive(Default)]
struct Jobs(HashMap<u64, Arc<AtomicBool>>);
impl Drop for Jobs {
    fn drop(&mut self) {
        for cancelled in self.0.values() {
            cancelled.store(true, Ordering::Release);
        }
    }
}

struct Outputs {
    sender: SyncSender<Output>,
    pending: VecDeque<(u64, Instant)>,
    sequence: u64,
}
impl Outputs {
    fn push(&mut self, value: Value) -> Result<(), String> {
        if self.pending.len() >= MAX_OUTPUT {
            return Err("MCP output queue is full".into());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("MCP output sequence exhausted")?;
        self.sender
            .try_send(Output {
                sequence: self.sequence,
                value,
            })
            .map_err(|_| "MCP output unavailable")?;
        self.pending.push_back((self.sequence, Instant::now()));
        Ok(())
    }
    fn acknowledge(&mut self, sequence: u64, success: bool) -> Result<(), String> {
        if !success || self.pending.pop_front().map(|(id, _)| id) != Some(sequence) {
            return Err("MCP output failed".into());
        }
        Ok(())
    }
    fn expired(&self, now: Instant) -> bool {
        self.pending
            .front()
            .is_some_and(|(_, started)| now.duration_since(*started) >= PIPE_DEADLINE)
    }
}

/// Caller must exit this dedicated process on return; pipe threads are detached.
pub fn run_from_environment() -> Result<(), String> {
    let client = Arc::new(Client::from_environment()?);
    let (sender, events) = mpsc::sync_channel(32);
    let input_sender = sender.clone();
    thread::Builder::new()
        .name("team-mcp-stdin".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            let mut reader = stdin.lock();
            let mut bytes = [0u8; 4096];
            loop {
                let event = match reader.read(&mut bytes) {
                    Ok(0) => Event::End,
                    Ok(count) => Event::Input(bytes[..count].to_vec()),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => Event::InputFailed,
                };
                let done = !matches!(event, Event::Input(_));
                if input_sender.send(event).is_err() || done {
                    break;
                }
            }
        })
        .map_err(|_| "Cannot start MCP input")?;
    let (output_sender, output_receiver) = mpsc::sync_channel::<Output>(MAX_OUTPUT);
    let output_events = sender.clone();
    thread::Builder::new()
        .name("team-mcp-stdout".into())
        .spawn(move || {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            while let Ok(output) = output_receiver.recv() {
                let success = team_mcp_stdio::write_message(&mut writer, &output.value).is_ok();
                if output_events
                    .send(Event::Output(output.sequence, success))
                    .is_err()
                    || !success
                {
                    break;
                }
            }
        })
        .map_err(|_| "Cannot start MCP output")?;

    let mut session = Session::default();
    let mut framer = Framer::default();
    let mut jobs = Jobs::default();
    let mut output = Outputs {
        sender: output_sender,
        pending: VecDeque::new(),
        sequence: 0,
    };
    let started = Instant::now();
    loop {
        let event = events.recv_timeout(Duration::from_millis(50));
        let now = Instant::now();
        if framer.expired(now)
            || output.expired(now)
            || (!session.is_ready() && now.duration_since(started) >= INITIALIZE_DEADLINE)
        {
            return Err("MCP pipe or initialization deadline expired".into());
        }
        match event {
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) | Ok(Event::InputFailed) => {
                return Err("MCP input unavailable".into());
            }
            Ok(Event::End) => {
                return if framer.bytes.is_empty() {
                    Ok(())
                } else {
                    Err("Incomplete MCP input".into())
                };
            }
            Ok(Event::Output(sequence, success)) => output.acknowledge(sequence, success)?,
            Ok(Event::Completed { id, ticket, result }) => {
                jobs.0.remove(&ticket);
                if let Some(response) = session.complete(&id, ticket, result) {
                    output.push(response)?;
                }
            }
            Ok(Event::Input(bytes)) => {
                for frame in framer.feed(&bytes, now)? {
                    match session.receive(&frame) {
                        Route::Reply(reply) => output.push(reply)?,
                        Route::Ignore => {}
                        Route::Cancel { ticket, .. } => {
                            if let Some(cancelled) = jobs.0.get(&ticket) {
                                cancelled.store(true, Ordering::Release);
                            }
                        }
                        Route::Dispatch {
                            id,
                            ticket,
                            operation,
                        } => {
                            if jobs.0.len() >= MAX_JOBS {
                                if let Some(reply) = session.complete(
                                    &id,
                                    ticket,
                                    Err(BackendError::Failed(
                                        "Team helper is busy; request was not sent".into(),
                                    )),
                                ) {
                                    output.push(reply)?;
                                }
                                continue;
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            jobs.0.insert(ticket, cancelled.clone());
                            let worker_client = client.clone();
                            let worker_sender = sender.clone();
                            let worker_id = id.clone();
                            let spawn = thread::Builder::new().name("team-mcp-call".into()).spawn(
                                move || {
                                    let result = worker_client
                                        .call_with_cancel(operation, &cancelled)
                                        .unwrap_or_else(|error| Err(BackendError::Failed(error)));
                                    let _ = worker_sender.send(Event::Completed {
                                        id: worker_id,
                                        ticket,
                                        result,
                                    });
                                },
                            );
                            if spawn.is_err() {
                                jobs.0.remove(&ticket);
                                if let Some(reply) = session.complete(
                                    &id,
                                    ticket,
                                    Err(BackendError::Failed(
                                        "Cannot start team request; nothing was sent".into(),
                                    )),
                                ) {
                                    output.push(reply)?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_limits_and_absolute_partial_deadlines_do_not_reject_idle_pipes() {
        let now = Instant::now();
        let mut framer = Framer::default();
        assert!(!framer.expired(now + PIPE_DEADLINE));
        assert!(framer.feed(b"{", now).unwrap().is_empty());
        assert!(
            framer
                .feed(b"}", now + PIPE_DEADLINE / 2)
                .unwrap()
                .is_empty()
        );
        assert!(framer.feed(b"\n", now + PIPE_DEADLINE).is_err());
        let mut framer = Framer::default();
        assert_eq!(
            framer.feed(b"{}\n{}\r\nx", now).unwrap(),
            vec![b"{}\n".to_vec(), b"{}\r\n".to_vec()]
        );
        assert!(framer.expired(now + PIPE_DEADLINE));
        let mut framer = Framer::default();
        assert!(
            framer
                .feed(&vec![b'x'; team_mcp_stdio::MAX_FRAME_BYTES], now)
                .unwrap()
                .is_empty()
        );
        assert!(framer.feed(b"\n", now).is_err());
    }
    #[test]
    fn output_capacity_ack_order_and_shutdown_cancellation_are_bounded() {
        let (sender, _receiver) = mpsc::sync_channel(MAX_OUTPUT);
        let mut output = Outputs {
            sender,
            pending: VecDeque::new(),
            sequence: 0,
        };
        for _ in 0..MAX_OUTPUT {
            output.push(Value::Null).unwrap();
        }
        assert!(output.push(Value::Null).is_err());
        assert!(output.expired(Instant::now() + PIPE_DEADLINE));
        assert!(output.acknowledge(99, true).is_err());
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut jobs = Jobs::default();
        jobs.0.insert(1, cancelled.clone());
        drop(jobs);
        assert!(cancelled.load(Ordering::Acquire));
    }
}
