//! `tessel-host serve`: accepts this user's connections on the private socket and answers them
//! with the protocol. Threads, not an async runtime: a few clients, each with its own reader.
use crate::log::Log;
use crate::runtime::{Runtime, bind, is_this_user, peer_uid, random_bytes, runtime, try_lock};
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tessel_host_protocol::{
    Budgets, ClientFrame, Connection, Epoch, ErrorCode, HostFrame, HostInfo, HostLimits,
    MAX_FRAME_BYTES, PersistenceMethod, Step, WireError, encode_frame, read_frame, wire_error,
};

const MAX_CLIENTS: usize = 16;
/// A connection that has not said hello by then is closed; after hello there is no limit.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const READY_TIMEOUT_MS: libc::c_int = 10_000;

enum Detached {
    /// The process that was started: exits with this once the daemon is ready (or not).
    Parent(i32),
    /// The daemon, in a new session, told to write one byte on this pipe when ready.
    Child(libc::c_int),
}

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// Must run before any thread exists.
fn detach() -> io::Result<Detached> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: fds has room for the two descriptors pipe2 writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(last_error());
    }
    // SAFETY: still single-threaded, so the child may go on running Rust code.
    match unsafe { libc::fork() } {
        -1 => Err(last_error()),
        0 => {
            // SAFETY: plain system calls on our own descriptors and a static path.
            unsafe {
                libc::close(fds[0]);
                libc::setsid();
                libc::chdir(c"/".as_ptr());
            }
            Ok(Detached::Child(fds[1]))
        }
        _ => {
            // SAFETY: our own descriptors; poll/read write only into locals.
            let code = unsafe {
                libc::close(fds[1]);
                let mut poll = libc::pollfd {
                    fd: fds[0],
                    events: libc::POLLIN,
                    revents: 0,
                };
                let mut byte = 0u8;
                let ready = libc::poll(&mut poll, 1, READY_TIMEOUT_MS) == 1
                    && libc::read(fds[0], (&mut byte as *mut u8).cast(), 1) == 1
                    && byte == b'k';
                libc::close(fds[0]);
                if ready { 0 } else { 1 }
            };
            Ok(Detached::Parent(code))
        }
    }
}

fn report(ready: &mut Option<libc::c_int>, byte: u8) {
    if let Some(fd) = ready.take() {
        // SAFETY: our own pipe end, written once and closed.
        unsafe {
            libc::write(fd, (&byte as *const u8).cast(), 1);
            libc::close(fd);
        }
    }
}

fn quiet_stdio() {
    // SAFETY: open/dup2/close on descriptors this process owns.
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
        if null >= 0 {
            for fd in 0..3 {
                libc::dup2(null, fd);
            }
            if null > 2 {
                libc::close(null);
            }
        }
    }
}

pub fn serve(daemonize: bool, method: PersistenceMethod) -> i32 {
    // SAFETY: process-wide settings, before any thread: files are private, and the end of the
    // SSH session that started us is not a reason to stop.
    unsafe {
        libc::umask(0o077);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("tessel-host: {error}");
            return 1;
        }
    };
    let mut ready = None;
    if daemonize {
        match detach() {
            Ok(Detached::Parent(code)) => return code,
            Ok(Detached::Child(fd)) => ready = Some(fd),
            Err(error) => {
                eprintln!("tessel-host: {error}");
                return 1;
            }
        }
    }
    match run(&runtime, method, &mut ready, daemonize) {
        Ok(()) => 0,
        Err(error) => {
            report(&mut ready, b'e');
            eprintln!("tessel-host: {error}");
            1
        }
    }
}

fn run(
    runtime: &Runtime,
    method: PersistenceMethod,
    ready: &mut Option<libc::c_int>,
    daemonize: bool,
) -> io::Result<()> {
    let Some(lock) = try_lock(runtime)? else {
        // Another daemon of this version serves this user: nothing to do.
        report(ready, b'k');
        return Ok(());
    };
    let listener = bind(runtime, &lock)?;
    let log = Arc::new(Log::new(runtime.log.clone()));
    let limits = HostLimits {
        capabilities: Vec::new(),
        budgets: Budgets::SLOW_HOST,
        epoch: Epoch::from_bytes(random_bytes()?),
        info: HostInfo {
            version: crate::VERSION.to_owned(),
            arch: crate::arch().to_owned(),
            pid: std::process::id(),
            persistence: crate::persistence::describe(method),
        },
    };
    log.event("serve", &[("pid", u64::from(std::process::id()))]);
    report(ready, b'k');
    if daemonize {
        quiet_stdio();
    }
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        if !peer_uid(&stream).is_ok_and(is_this_user) {
            log.event("peer_refused", &[]);
            continue;
        }
        if active.load(Ordering::SeqCst) >= MAX_CLIENTS {
            let _ = send(
                &mut stream,
                &HostFrame::connection_error(WireError::new(ErrorCode::Busy)),
                MAX_FRAME_BYTES,
            );
            continue;
        }
        active.fetch_add(1, Ordering::SeqCst);
        let (limits, log, active) = (limits.clone(), log.clone(), active.clone());
        std::thread::spawn(move || {
            log.event("client_open", &[]);
            client(stream, limits);
            log.event("client_close", &[]);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
    drop(lock);
    Ok(())
}

fn send(stream: &mut UnixStream, frame: &HostFrame, limit: usize) -> io::Result<()> {
    let bytes = encode_frame(frame, limit)?;
    stream.write_all(&bytes)
}

fn client(stream: UnixStream, limits: HostLimits) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = stream;
    let _ = reader.set_read_timeout(Some(HELLO_TIMEOUT));
    let mut connection = Connection::new(limits);
    let mut limit = MAX_FRAME_BYTES;
    loop {
        let frame = match read_frame::<ClientFrame>(&mut reader, limit) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => {
                if let Some(refusal) = wire_error(&error) {
                    let _ = send(
                        &mut writer,
                        &HostFrame::connection_error(refusal.clone()),
                        limit,
                    );
                }
                break;
            }
        };
        let sent = match connection.on_frame(frame) {
            Ok(Step::Reply(reply)) => send(&mut writer, &reply, limit),
            // No capability is offered before gate 3: a dispatch cannot happen yet.
            Ok(Step::Dispatch(dispatch)) => match connection.finish(
                dispatch.request_id,
                Err(WireError::new(ErrorCode::UnsupportedKind)),
            ) {
                Some(reply) => send(&mut writer, &reply, limit),
                None => Ok(()),
            },
            Ok(Step::Ack(_)) => Ok(()),
            Err(fatal) => {
                let _ = send(&mut writer, &fatal.frame(), limit);
                break;
            }
        };
        if sent.is_err() {
            break;
        }
        if let Some(agreed) = connection.negotiated() {
            limit = agreed.budgets.frame_bytes as usize;
            let _ = reader.set_read_timeout(None);
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}
