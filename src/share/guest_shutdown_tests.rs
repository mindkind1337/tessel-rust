use super::*;
use std::net::Shutdown;
fn wait_finished(worker: thread::JoinHandle<()>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        assert!(
            Instant::now() < deadline,
            "cancelled network worker did not exit"
        );
        thread::sleep(Duration::from_millis(5));
    }
    worker.join().unwrap();
}
fn invitation() -> (tempfile::TempDir, Host) {
    let dir = tempfile::Builder::new()
        .prefix("tessel-cancel-fixture-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let host = Host::start(
        dir.path(),
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    (dir, host)
}
#[test]
fn guest_shutdown_leave_and_drop_interrupt_blocked_tls_even_with_full_queue() {
    let (_dir, host) = invitation();
    for drop_session in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut invite = host.invitation.clone();
        invite.address = listener.local_addr().unwrap();
        let (ready, ready_rx) = mpsc::sync_channel(1);
        let (closed, closed_rx) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0u8; 4096];
            assert!(stream.read(&mut buffer).unwrap() > 0); // TLS hello reached the silent server.
            ready.send(()).unwrap();
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = closed.send(());
        });
        let mut guest = GuestSession::connect(invite, "Synthetic".into(), "d4".repeat(32));
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let replies: Vec<_> = (0..32)
            .map(|_| guest.request(json!({"op":"workspace.snapshot"})).unwrap())
            .collect();
        assert!(guest.request(json!({"op":"workspace.snapshot"})).is_err());
        assert!(
            guest
                .call_sync(json!({"op":"workspace.snapshot"}), Duration::from_millis(5))
                .is_err()
        );
        let worker = guest.worker.take().unwrap();
        let state = guest.state.clone();
        let stop = guest.stop.clone();
        let started = Instant::now();
        if drop_session {
            drop(guest);
        } else {
            guest.leave();
            assert!(guest.request(json!({"op":"workspace.snapshot"})).is_err());
        }
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "UI must not wait for the TLS deadline"
        );
        closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        wait_finished(worker);
        server.join().unwrap();
        assert!(
            matches!(&state.lock().unwrap().phase, GuestPhase::Ended {reason, ..} if reason == "left")
        );
        assert!(active_state(&state, &stop).is_none());
        assert!(
            replies
                .iter()
                .all(|r| matches!(r.try_recv(), Err(mpsc::TryRecvError::Disconnected)))
        );
    }
}
#[test]
fn guest_shutdown_registration_after_cancel_never_installs_or_sends_tls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let stop = GuestStop::default();
    stop.cancel();
    assert!(stop.register(&socket).is_err());
    drop(socket);
    assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
}
#[test]
fn guest_shutdown_registered_socket_is_interrupted_and_unregister_releases_clone() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let stop = GuestStop::default();
    stop.register(&socket).unwrap();
    stop.cancel();
    assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
    let _ = socket.shutdown(Shutdown::Both);
    assert!(stop.register(&socket).is_err());

    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let stop = GuestStop::default();
    stop.register(&socket).unwrap();
    drop(socket);
    {
        let _lifetime = SocketLifetime(&stop);
    }
    assert_eq!(
        peer.read(&mut [0u8; 1]).unwrap(),
        0,
        "worker exit must release the cancellation clone"
    );
}
#[test]
fn guest_shutdown_real_rpc_wait_keeps_left_phase_and_cleans_host() {
    let (_dir, host) = invitation();
    host.publish_tasks(vec![]);
    let mut guest =
        GuestSession::connect(host.invitation.clone(), "Synthetic".into(), "e5".repeat(32));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(LiveShareAsk::Join { key, .. }) = host.snapshot().asks.first() {
            host.answer_ask(AskAnswer::Join {
                key: key.clone(),
                allow: true,
                projects: vec![1],
                rights: ShareRights::collaborate(),
                trust: false,
            });
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    while !matches!(guest.snapshot().phase, GuestPhase::Joined { .. }) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let reply = guest.request(json!({"op":"tasks.mutate","action":"add","projectId":1,"title":"Synthetic","column":"todo"})).unwrap();
    let command = loop {
        if let Some(command) = host.drain_commands().into_iter().find_map(|c| match c {
            HostCommand::TaskMutation(command) => Some(command),
            _ => None,
        }) {
            break command;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    };
    let worker = guest.worker.take().unwrap();
    let started = Instant::now();
    guest.leave();
    assert!(started.elapsed() < Duration::from_millis(250));
    wait_finished(worker);
    assert!(matches!(
        reply.recv_timeout(Duration::from_secs(1)),
        Ok(Err(GuestRpcError::Transport(_)))
    ));
    assert!(matches!(guest.snapshot().phase, GuestPhase::Ended {reason,..} if reason == "left"));
    let deadline = Instant::now() + Duration::from_secs(4);
    while !host.snapshot().guests.is_empty() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    command.reject();
}

fn empty_state() -> Arc<Mutex<GuestSnapshot>> {
    Arc::new(Mutex::new(GuestSnapshot {
        phase: GuestPhase::Connecting,
        endpoint: "synthetic".into(),
        rights: ShareRights::view_only(),
        projects: vec![],
        panes: vec![],
        pages: vec![],
        host_active_project: None,
        host_active_pane: None,
        last_pane_output: None,
        last_error: String::new(),
    }))
}
#[test]
fn guest_shutdown_never_joins_a_worker_held_after_ui_return() {
    for dropping in [false, true] {
        let (release, gate) = mpsc::sync_channel(1);
        let (finished, finish_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
            finished.send(()).unwrap();
        });
        let (tx, _rx) = mpsc::sync_channel(1);
        let mut guest = GuestSession {
            generation: 1,
            state: empty_state(),
            tx,
            stop: Arc::new(GuestStop::default()),
            worker: Some(worker),
        };
        let (returned, return_rx) = mpsc::sync_channel(1);
        let ui = thread::spawn(move || {
            if dropping {
                drop(guest);
            } else {
                guest.leave();
            }
            returned.send(()).unwrap();
        });
        let outcome = return_rx.recv_timeout(Duration::from_secs(1));
        // Always release before asserting, so an accidental join fails without hanging the suite.
        release.send(()).unwrap();
        ui.join().unwrap();
        finish_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            outcome,
            Ok(()),
            "leave/drop waited for a worker that was deliberately held"
        );
    }
}
#[test]
fn guest_shutdown_cancelled_before_worker_start_makes_no_connection() {
    let (_dir, host) = invitation();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut invite = host.invitation.clone();
    invite.address = listener.local_addr().unwrap();
    let stop = Arc::new(GuestStop::default());
    let state = empty_state();
    stop.cancel();
    mark_left(&state);
    let (_tx, rx) = mpsc::sync_channel(1);
    run_guest_worker(
        invite,
        "Synthetic".into(),
        "f6".repeat(32),
        state.clone(),
        rx,
        stop,
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(
        matches!(&state.lock().unwrap().phase, GuestPhase::Ended {reason,..} if reason == "left")
    );
}

#[test]
fn guest_shutdown_read_observes_cancel_without_socket_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (_peer, _) = listener.accept().unwrap();
    // Deliberately do not register a cancellation clone: this exercises the
    // fallback when an OS does not wake an already pending read on shutdown.
    let stop = Arc::new(GuestStop::default());
    let mut stream = DeadlineStream::new(socket, Duration::from_secs(15));
    stream.stop = Some(stop.clone());
    let (ready, ready_rx) = mpsc::sync_channel(1);
    let (done, done_rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        ready.send(()).unwrap();
        done.send(stream.read(&mut [0; 1]).unwrap_err().kind())
            .unwrap();
    });
    ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    thread::sleep(Duration::from_millis(150));
    stop.cancel();
    assert_eq!(
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        std::io::ErrorKind::ConnectionAborted
    );
    worker.join().unwrap();
}
#[test]
fn guest_shutdown_short_io_slices_preserve_data_and_global_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    let mut stream = DeadlineStream::new(socket, Duration::from_secs(2));
    stream.stop = Some(Arc::new(GuestStop::default()));
    let sender = thread::spawn(move || {
        thread::sleep(Duration::from_millis(350));
        peer.write_all(b"x").unwrap();
        peer
    });
    let mut byte = [0];
    assert_eq!(stream.read(&mut byte).unwrap(), 1);
    assert_eq!(byte, b"x"[0..1]);
    let _peer = sender.join().unwrap();
    stream.reset_deadline(Duration::from_millis(250));
    let start = Instant::now();
    assert_eq!(
        stream.read(&mut byte).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    stream.stop.as_ref().unwrap().cancel();
    assert_eq!(
        stream.write(b"no").unwrap_err().kind(),
        std::io::ErrorKind::ConnectionAborted
    );
}
