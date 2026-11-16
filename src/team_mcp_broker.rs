//! Bounded local broker. This module never executes tools or mutates Workspace.
//! Runtime callers must also recheck each trusted Binding against current pane state.
pub use crate::team_mcp_auth::Binding;
use crate::team_mcp_auth::{Authority, Credential, VerifiedRequest};
use crate::team_mcp_stdio::{BackendError, Operation};
use crate::team_mcp_wire::{decode, encode, read_packet_while, write_packet_while};
use crate::teams_relay::{SignedTeamRequest, now_ms};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(crate) const ENV_ENDPOINT: &str = "TESSEL_TEAM_MCP_ENDPOINT";

#[derive(Clone, Copy)]
struct Limits {
    peers: usize,
    queue: usize,
    deadline: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            peers: 4,
            queue: 16,
            deadline: Duration::from_secs(10),
        }
    }
}

struct Shared {
    authority: Mutex<Authority>,
    stopped: AtomicBool,
    active: AtomicUsize,
    queued: AtomicUsize,
    sockets: Mutex<HashMap<u64, TcpStream>>,
}

pub struct Client {
    endpoint: SocketAddr,
    credential: Credential,
    timeout: Duration,
}

/// Keep this owner with the actual child transport, never in persisted pane state.
/// A replaced transport can drop without revoking its successor's capability.
pub struct Launch {
    client: Client,
    binding: Binding,
    shared: Arc<Shared>,
}

impl Launch {
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    pub fn revoke(&self) {
        if let Ok(mut authority) = self.shared.authority.lock()
            && authority.current(&self.binding)
        {
            authority.revoke(self.binding.pane);
        }
    }
}

impl Drop for Launch {
    fn drop(&mut self) {
        self.revoke();
    }
}

impl Client {
    /// Scoped credentials go only to the child environment, after profile setup.
    /// Do not log/debug this Command or forward these variables to remote hosts.
    pub fn apply_environment(&self, command: &mut std::process::Command) {
        command.env(ENV_ENDPOINT, self.endpoint.to_string());
        self.credential.apply_environment(command);
    }

    pub(crate) fn from_environment() -> Result<Self, String> {
        Self::from_values(|name| {
            std::env::var(name).map_err(|_| "Missing team helper environment".into())
        })
    }

    fn from_values(mut read: impl FnMut(&str) -> Result<String, String>) -> Result<Self, String> {
        use crate::team_mcp_auth::{ENV_GENERATION, ENV_KEY, ENV_PANE};
        fn number(text: String) -> Result<u64, String> {
            let value = text
                .parse::<u64>()
                .map_err(|_| "Invalid team helper identity")?;
            if text.len() > 20 || text != value.to_string() {
                return Err("Invalid team helper identity".into());
            }
            Ok(value)
        }
        let endpoint = read(ENV_ENDPOINT)?;
        if endpoint.len() > 32 {
            return Err("Invalid local team endpoint".into());
        }
        let endpoint: SocketAddr = endpoint
            .parse()
            .map_err(|_| "Invalid local team endpoint")?;
        if endpoint.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) || endpoint.port() == 0 {
            return Err("Invalid local team endpoint".into());
        }
        let pane = number(read(ENV_PANE)?)?;
        let generation = number(read(ENV_GENERATION)?)?;
        let key = zeroize::Zeroizing::new(read(ENV_KEY)?);
        let credential = Credential::import(pane, generation, &key)?;
        Ok(Self {
            endpoint,
            credential,
            timeout: Limits::default().deadline,
        })
    }

    /// One exchange only: failures after sending may be uncertain and are never retried.
    pub fn call(&self, operation: Operation) -> Result<Result<Value, BackendError>, String> {
        self.call_with_cancel(operation, &AtomicBool::new(false))
    }

    pub fn call_with_cancel(
        &self,
        operation: Operation,
        cancelled: &AtomicBool,
    ) -> Result<Result<Value, BackendError>, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("Team request cancelled before sending".into());
        }
        if self.endpoint.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
            || self.endpoint.port() == 0
        {
            return Err("Invalid local team endpoint".into());
        }
        let deadline = Instant::now() + self.timeout;
        let request = self.credential.request(operation, now_ms())?;
        let mut stream =
            TcpStream::connect_timeout(&self.endpoint, self.timeout.min(Duration::from_secs(2)))
                .map_err(|_| "Local team broker is unavailable".to_owned())?;
        write_packet_while(&mut stream, &encode(&request)?, deadline, || {
            !cancelled.load(Ordering::Acquire)
        })?;
        let response: SignedTeamRequest = decode(&read_packet_while(&mut stream, deadline,||!cancelled.load(Ordering::Acquire))
            .map_err(|_| "Team response unavailable; operation may already have completed. Do not retry automatically.".to_owned())?)?;
        self.credential.response(&request, &response, now_ms())
    }
}

pub struct Pending {
    verified: VerifiedRequest,
    shared: Arc<Shared>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    response: SyncSender<SignedTeamRequest>,
    peer: TcpStream,
    queued: bool,
    finished: bool,
}

impl Pending {
    pub fn binding(&self) -> &Binding {
        &self.verified.binding
    }
    pub fn operation(&self) -> &Operation {
        &self.verified.operation
    }

    pub fn is_current(&self) -> bool {
        !self.shared.stopped.load(Ordering::Acquire)
            && !self.cancelled.load(Ordering::Acquire)
            && Instant::now() < self.deadline
            && peer_open(&self.peer)
            && self
                .shared
                .authority
                .lock()
                .is_ok_and(|authority| authority.current(self.binding()))
    }

    fn dequeue(&mut self) {
        if self.queued {
            self.queued = false;
            self.shared.queued.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub fn finish(mut self, result: Result<Value, BackendError>) -> Result<(), String> {
        if !self.is_current() {
            return Err("Team request expired, was cancelled or revoked".into());
        }
        let response = self
            .shared
            .authority
            .lock()
            .map_err(|_| "Team authority unavailable")?
            .response(&self.verified, result, now_ms())?;
        if Instant::now() >= self.deadline || self.cancelled.load(Ordering::Acquire) {
            return Err("Team result missed its deadline; outcome may be unknown".into());
        }
        self.response
            .try_send(response)
            .map_err(|_| "Team response receiver is unavailable")?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.dequeue();
        if !self.finished {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

pub struct Broker {
    endpoint: SocketAddr,
    shared: Arc<Shared>,
    incoming: Receiver<Pending>,
    listener: Option<JoinHandle<()>>,
    timeout: Duration,
}

impl Broker {
    pub fn bind() -> Result<Self, String> {
        Self::bind_with_limits(Limits::default())
    }

    fn bind_with_limits(limits: Limits) -> Result<Self, String> {
        if limits.peers == 0 || limits.queue == 0 || limits.deadline.is_zero() {
            return Err("Invalid team broker limits".into());
        }
        // Exactly one bind; no fallback ports, interfaces or endpoints.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Cannot bind local team broker")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot configure local team broker")?;
        let endpoint = listener
            .local_addr()
            .map_err(|_| "Cannot read local team endpoint")?;
        let (sender, incoming) = mpsc::sync_channel(limits.queue);
        let shared = Arc::new(Shared {
            authority: Mutex::new(Authority::default()),
            stopped: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            queued: AtomicUsize::new(0),
            sockets: Mutex::new(HashMap::new()),
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("team-broker".into())
            .spawn(move || accept_loop(listener, sender, worker_shared, limits))
            .map_err(|_| "Cannot start local team broker")?;
        Ok(Self {
            endpoint,
            shared,
            incoming,
            listener: Some(worker),
            timeout: limits.deadline,
        })
    }

    pub fn issue(&self, pane: u64, project: u64, cwd: &Path) -> Result<Client, String> {
        self.issue_inner(pane, project, cwd)
            .map(|(client, _)| client)
    }

    fn issue_inner(
        &self,
        pane: u64,
        project: u64,
        cwd: &Path,
    ) -> Result<(Client, Binding), String> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err("Team broker stopped".into());
        }
        let mut authority = self
            .shared
            .authority
            .lock()
            .map_err(|_| "Team authority unavailable")?;
        let credential = authority.issue(pane, project, cwd)?;
        let binding = authority.binding(pane).ok_or("Team binding unavailable")?;
        Ok((
            Client {
                endpoint: self.endpoint,
                credential,
                timeout: self.timeout,
            },
            binding,
        ))
    }

    pub fn launch(&self, pane: u64, project: u64, cwd: &Path) -> Result<Launch, String> {
        let (client, binding) = self.issue_inner(pane, project, cwd)?;
        Ok(Launch {
            client,
            binding,
            shared: self.shared.clone(),
        })
    }

    pub fn bindings(&self) -> Result<Vec<Binding>, String> {
        self.shared
            .authority
            .lock()
            .map(|authority| authority.bindings())
            .map_err(|_| "Team authority unavailable".into())
    }

    pub fn revoke(&self, pane: u64) {
        if let Ok(mut authority) = self.shared.authority.lock() {
            authority.revoke(pane);
        }
    }

    pub fn try_next(&self) -> Option<Pending> {
        while let Ok(mut pending) = self.incoming.try_recv() {
            pending.dequeue();
            if pending.is_current() {
                return Some(pending);
            }
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn queued_for_test(&self) -> usize {
        self.shared.queued.load(Ordering::Acquire)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared
            .authority
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .revoke_all();
        for stream in self
            .shared
            .sockets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.listener.take() {
            let _ = worker.join();
        }
        while self.incoming.try_recv().is_ok() {}
    }
}

struct PeerGuard {
    shared: Arc<Shared>,
    id: u64,
}
impl Drop for PeerGuard {
    fn drop(&mut self) {
        if let Some(stream) = self
            .shared
            .sockets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id)
        {
            // Pending owns a probe clone: dropping this handle alone cannot close it.
            let _ = stream.shutdown(Shutdown::Both);
        }
        self.shared.active.fetch_sub(1, Ordering::AcqRel);
    }
}

fn accept_loop(
    listener: TcpListener,
    sender: SyncSender<Pending>,
    shared: Arc<Shared>,
    limits: Limits,
) {
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    let mut next_id = 0u64;
    while !shared.stopped.load(Ordering::Acquire) {
        let mut i = 0;
        while i < workers.len() {
            if workers[i].is_finished() {
                let _ = workers.swap_remove(i).join();
            } else {
                i += 1;
            }
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() || shared.active.load(Ordering::Acquire) >= limits.peers
                {
                    let _ = stream.shutdown(Shutdown::Both);
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                let Some(id) = next_id.checked_add(1) else {
                    break;
                };
                next_id = id;
                let Ok(copy) = stream.try_clone() else {
                    continue;
                };
                let Ok(mut sockets) = shared.sockets.lock() else {
                    break;
                };
                if shared.stopped.load(Ordering::Acquire) {
                    break;
                }
                sockets.insert(id, copy);
                drop(sockets);
                shared.active.fetch_add(1, Ordering::AcqRel);
                let guard = PeerGuard {
                    shared: shared.clone(),
                    id,
                };
                let sender = sender.clone();
                let deadline = Instant::now() + limits.deadline;
                if let Ok(worker) =
                    thread::Builder::new()
                        .name("team-peer".into())
                        .spawn(move || {
                            let connection = guard;
                            serve_peer(stream, sender, &connection.shared, deadline);
                        })
                {
                    workers.push(worker);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    shared.stopped.store(true, Ordering::Release);
    for stream in shared
        .sockets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
    {
        let _ = stream.shutdown(Shutdown::Both);
    }
    for worker in workers {
        let _ = worker.join();
    }
}

struct CancelOnExit(Arc<AtomicBool>);
impl Drop for CancelOnExit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn serve_peer(
    mut stream: TcpStream,
    sender: SyncSender<Pending>,
    shared: &Arc<Shared>,
    deadline: Instant,
) {
    let Ok(bytes) = read_packet_while(&mut stream, deadline, || {
        !shared.stopped.load(Ordering::Acquire)
    }) else {
        return;
    };
    let Ok(request) = decode::<SignedTeamRequest>(&bytes) else {
        return;
    };
    let Ok(mut authority) = shared.authority.lock() else {
        return;
    };
    let Ok(verified) = authority.authenticate(&request, now_ms()) else {
        return;
    };
    drop(authority);
    if shared.stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
        return;
    }
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let Ok(peer) = stream.try_clone() else {
        return;
    };
    if !peer_open(&peer) {
        return;
    }
    let binding = verified.binding.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnExit(cancelled.clone());
    let (response, receiver) = mpsc::sync_channel(1);
    shared.queued.fetch_add(1, Ordering::AcqRel);
    let pending = Pending {
        verified,
        shared: shared.clone(),
        deadline,
        cancelled: cancelled.clone(),
        response,
        peer,
        queued: true,
        finished: false,
    };
    if sender.try_send(pending).is_err() {
        return;
    }
    loop {
        if shared.stopped.load(Ordering::Acquire)
            || cancelled.load(Ordering::Acquire)
            || !peer_open(&stream)
            || !shared
                .authority
                .lock()
                .is_ok_and(|authority| authority.current(&binding))
        {
            return;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return;
        };
        match receiver.recv_timeout(remaining.min(Duration::from_millis(20))) {
            Ok(response) => {
                if !shared
                    .authority
                    .lock()
                    .is_ok_and(|authority| authority.current(&binding))
                {
                    return;
                }
                let Ok(bytes) = encode(&response) else {
                    return;
                };
                let _ = write_packet_while(&mut stream, &bytes, deadline, || {
                    !shared.stopped.load(Ordering::Acquire)
                        && !cancelled.load(Ordering::Acquire)
                        && shared
                            .authority
                            .lock()
                            .is_ok_and(|authority| authority.current(&binding))
                });
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

// One packet per connection. EOF means cancellation; extra input is not a second request.
// The socket is nonblocking throughout the pending phase, including its cloned probe.
fn peer_open(stream: &TcpStream) -> bool {
    let mut byte = [0];
    matches!(stream.peek(&mut byte), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team_mcp_wire::{read_packet, write_packet};
    use serde_json::json;

    #[test]
    fn child_environment_is_scoped_and_rejects_remote_endpoints_and_malformed_credentials() {
        use crate::team_mcp_auth::{ENV_GENERATION, ENV_KEY, ENV_PANE};
        let broker = Broker::bind().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let launch = broker.launch(7, 9, &cwd).unwrap();
        let mut command = std::process::Command::new("synthetic-helper-not-executed");
        command.arg("--team-mcp-stdio");
        launch.client().apply_environment(&mut command);
        assert!(
            command
                .get_args()
                .eq([std::ffi::OsStr::new("--team-mcp-stdio")])
        );
        let values: HashMap<String, String> = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_str().unwrap().to_owned(),
                    value.unwrap().to_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(values.len(), 4);
        let import = |values: &HashMap<String, String>| {
            Client::from_values(|name| {
                values
                    .get(name)
                    .cloned()
                    .ok_or_else(|| "Missing fixture field".into())
            })
        };
        assert!(import(&values).is_ok());
        for (name, replacement) in [
            (ENV_ENDPOINT, "http://127.0.0.1:12"),
            (ENV_ENDPOINT, "192.0.2.1:12"),
            (ENV_ENDPOINT, "[::1]:12"),
            (ENV_ENDPOINT, "127.0.0.1:0"),
            (ENV_ENDPOINT, "localhost:12"),
            (ENV_PANE, "-1"),
            (ENV_PANE, "07"),
            (ENV_GENERATION, "0"),
            (ENV_GENERATION, "18446744073709551616"),
            (ENV_KEY, "not-a-key"),
            (ENV_KEY, "é"),
        ] {
            let mut changed = values.clone();
            changed.insert(name.into(), replacement.into());
            assert!(import(&changed).is_err());
        }
        for name in [ENV_ENDPOINT, ENV_PANE, ENV_GENERATION, ENV_KEY] {
            let mut missing = values.clone();
            missing.remove(name);
            assert!(import(&missing).is_err());
        }
    }

    fn broker(peers: usize, queue: usize, timeout: Duration) -> Broker {
        Broker::bind_with_limits(Limits {
            peers,
            queue,
            deadline: timeout,
        })
        .unwrap()
    }
    #[track_caller]
    fn wait(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "synthetic broker condition timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn next(broker: &Broker) -> Pending {
        let mut pending = None;
        wait(|| {
            pending = broker.try_next();
            pending.is_some()
        });
        pending.unwrap()
    }
    fn send_raw(broker: &Broker, request: &SignedTeamRequest) -> TcpStream {
        let mut stream = TcpStream::connect(broker.endpoint).unwrap();
        write_packet(
            &mut stream,
            &encode(request).unwrap(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        stream
    }

    #[test]
    fn signed_roundtrip_delivers_only_trusted_scope_without_executing_a_tool() {
        let root = tempfile::tempdir().unwrap();
        let broker = Broker::bind().unwrap();
        assert_eq!(
            broker.endpoint.ip(),
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        let client = broker.issue(10, 20, root.path()).unwrap();
        let worker = thread::spawn(move || {
            client.call(Operation::Call {
                name: "team_inbox".into(),
                arguments: json!({"project":999,"me":"somebody"}),
            })
        });
        let pending = next(&broker);
        assert_eq!(pending.binding().pane, 10);
        assert_eq!(pending.binding().project, 20);
        assert_eq!(pending.binding().cwd, root.path());
        assert!(matches!(pending.operation(),Operation::Call {name,..} if name == "team_inbox"));
        pending.finish(Ok(json!({"synthetic":true}))).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap().unwrap(),
            json!({"synthetic":true})
        );
        wait(|| broker.shared.active.load(Ordering::Acquire) == 0);
        assert_eq!(broker.shared.queued.load(Ordering::Acquire), 0);
    }

    #[test]
    fn rotation_revocation_and_dropped_requests_cannot_deliver_old_results() {
        let root = tempfile::tempdir().unwrap();
        let broker = broker(4, 4, Duration::from_secs(2));
        for replacement in [false, true] {
            let client = broker.issue(1, 2, root.path()).unwrap();
            let worker = thread::spawn(move || client.call(Operation::List));
            let pending = next(&broker);
            if replacement {
                let _new = broker.issue(1, 3, root.path()).unwrap();
            } else {
                broker.revoke(1);
            }
            assert!(!pending.is_current());
            assert!(pending.finish(Ok(json!([]))).is_err());
            assert!(worker.join().unwrap().is_err());
        }
        let client = broker.issue(1, 2, root.path()).unwrap();
        let worker = thread::spawn(move || client.call(Operation::List));
        drop(next(&broker));
        assert!(worker.join().unwrap().is_err());
    }

    #[test]
    fn disconnected_and_expired_peers_are_rejected_before_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let broker = broker(4, 4, Duration::from_millis(200));
        let client = broker.issue(1, 2, root.path()).unwrap();
        let request = client
            .credential
            .request(Operation::List, now_ms())
            .unwrap();
        let stream = send_raw(&broker, &request);
        wait(|| broker.shared.queued.load(Ordering::Acquire) == 1);
        stream.shutdown(Shutdown::Both).unwrap();
        drop(stream);
        wait(|| broker.shared.active.load(Ordering::Acquire) == 0);
        assert!(broker.try_next().is_none());
        let worker = thread::spawn(move || client.call(Operation::List));
        let pending = next(&broker);
        assert!(worker.join().unwrap().is_err());
        wait(|| !pending.is_current());
        assert!(pending.finish(Ok(json!([]))).is_err());
        assert!(broker.try_next().is_none());
    }

    #[test]
    fn replay_and_invalid_signatures_never_enter_the_dispatch_queue() {
        let root = tempfile::tempdir().unwrap();
        let broker = broker(4, 4, Duration::from_secs(2));
        let client = broker.issue(1, 2, root.path()).unwrap();
        let request = client
            .credential
            .request(Operation::List, now_ms())
            .unwrap();
        let mut stream = send_raw(&broker, &request);
        next(&broker).finish(Ok(json!([]))).unwrap();
        let response = decode::<SignedTeamRequest>(
            &read_packet(&mut stream, Instant::now() + Duration::from_secs(1)).unwrap(),
        )
        .unwrap();
        assert!(
            client
                .credential
                .response(&request, &response, now_ms())
                .unwrap()
                .is_ok()
        );
        for invalid in [
            request.clone(),
            {
                let mut forged = client
                    .credential
                    .request(Operation::List, now_ms())
                    .unwrap();
                forged.signature_hex = "00".repeat(32);
                forged
            },
            response,
        ] {
            let mut stream = send_raw(&broker, &invalid);
            assert!(read_packet(&mut stream, Instant::now() + Duration::from_secs(1)).is_err());
            assert!(broker.try_next().is_none());
        }
    }

    #[test]
    fn bounded_queue_and_connection_limit_do_not_displace_an_accepted_request() {
        let root = tempfile::tempdir().unwrap();
        let broker = broker(2, 1, Duration::from_secs(2));
        let client = Arc::new(broker.issue(1, 2, root.path()).unwrap());
        let first = client.clone();
        let first = thread::spawn(move || first.call(Operation::List));
        wait(|| broker.shared.queued.load(Ordering::Acquire) == 1);
        let second = client.clone();
        let second = thread::spawn(move || second.call(Operation::List));
        assert!(second.join().unwrap().is_err());
        let accepted = next(&broker);
        accepted.finish(Ok(json!([]))).unwrap();
        assert_eq!(first.join().unwrap().unwrap().unwrap(), json!([]));
        wait(|| broker.shared.active.load(Ordering::Acquire) == 0);
        let a = TcpStream::connect(broker.endpoint).unwrap();
        let b = TcpStream::connect(broker.endpoint).unwrap();
        wait(|| broker.shared.active.load(Ordering::Acquire) == 2);
        let mut rejected = TcpStream::connect(broker.endpoint).unwrap();
        assert!(read_packet(&mut rejected, Instant::now() + Duration::from_secs(1)).is_err());
        assert!(broker.try_next().is_none());
        drop(a);
        drop(b);
    }

    #[test]
    fn shutdown_closes_partial_and_pending_connections_and_revokes_held_requests() {
        let root = tempfile::tempdir().unwrap();
        let broker = broker(4, 4, Duration::from_secs(10));
        let client = broker.issue(1, 2, root.path()).unwrap();
        let worker = thread::spawn(move || client.call(Operation::List));
        let pending = next(&broker);
        let mut partial = TcpStream::connect(broker.endpoint).unwrap();
        use std::io::Write as _;
        partial.write_all(&[0, 0]).unwrap();
        wait(|| broker.shared.active.load(Ordering::Acquire) == 2);
        let shared = broker.shared.clone();
        let start = Instant::now();
        drop(broker);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!pending.is_current());
        assert!(pending.finish(Ok(json!([]))).is_err());
        assert!(worker.join().unwrap().is_err());
        assert!(read_packet(&mut partial, Instant::now() + Duration::from_secs(1)).is_err());
        assert_eq!(shared.active.load(Ordering::Acquire), 0);
        assert_eq!(shared.queued.load(Ordering::Acquire), 0);
    }

    #[test]
    fn expired_queued_requests_close_the_connection_before_main_thread_dequeues_them() {
        use std::io::Read as _;
        let root = tempfile::tempdir().unwrap();
        let broker = broker(2, 2, Duration::from_millis(150));
        let client = broker.issue(1, 2, root.path()).unwrap();
        let request = client
            .credential
            .request(Operation::List, now_ms())
            .unwrap();
        let mut stream = send_raw(&broker, &request);
        wait(|| broker.shared.queued.load(Ordering::Acquire) == 1);
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        let closed = match stream.read(&mut byte) {
            Ok(0) => true,
            Err(error) => matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ),
            _ => false,
        };
        assert!(
            closed,
            "deadline must close TCP even while Pending retains a probe clone"
        );
        wait(|| broker.shared.active.load(Ordering::Acquire) == 0);
        assert_eq!(broker.shared.queued.load(Ordering::Acquire), 1);
        assert!(broker.try_next().is_none());
        assert_eq!(broker.shared.queued.load(Ordering::Acquire), 0);
    }

    #[test]
    fn client_cancellation_stops_an_exchange_without_waiting_for_its_deadline_or_retrying() {
        let root = tempfile::tempdir().unwrap();
        let broker = broker(2, 2, Duration::from_secs(10));
        let client = broker.issue(1, 2, root.path()).unwrap();
        assert!(
            client
                .call_with_cancel(Operation::List, &AtomicBool::new(true))
                .is_err()
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let worker = thread::spawn(move || client.call_with_cancel(Operation::List, &signal));
        let pending = next(&broker);
        let start = Instant::now();
        cancelled.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_err());
        wait(|| !pending.is_current());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(pending.finish(Ok(json!([]))).is_err());
        assert!(broker.try_next().is_none());
    }
}
