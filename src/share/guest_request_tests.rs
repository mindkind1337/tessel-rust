use super::*;
#[test]
fn guest_request_full_queue_returns_immediately_and_retains_reply_receiver() {
    let state = GuestSnapshot {
        phase: GuestPhase::Joined {
            sas_code: "123456".into(),
            host_name: "Fixture".into(),
        },
        endpoint: "fixture".into(),
        rights: ShareRights::view_only(),
        projects: vec![(1, "Fixture".into())],
        panes: vec![],
        pages: vec![],
        host_active_project: Some(1),
        host_active_pane: None,
        last_pane_output: None,
        entries: vec![],
        current_path: String::new(),
        downloaded_file: None,
        last_error: String::new(),
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let session = GuestSession {
        generation: 1,
        state: Arc::new(Mutex::new(state)),
        tx,
        stop: Arc::new(AtomicBool::new(false)),
        worker: None,
    };
    let reply = session
        .request(json!({"op":"tasks.list","projectId":1}))
        .unwrap();
    // Release the occupied slot even if a regression blocks the sender, so failure is bounded.
    let request = thread::scope(|scope| {
        let (done, result) = mpsc::sync_channel(1);
        let session = &session;
        scope.spawn(move || {
            let _ = done.send(
                session
                    .request(json!({"op":"tasks.list","projectId":1}))
                    .is_err(),
            );
        });
        let outcome = result.recv_timeout(Duration::from_secs(2));
        let request = rx.try_recv().unwrap();
        assert_eq!(outcome, Ok(true));
        request
    });
    let GuestRpcReq::Call {
        payload,
        reply: Some(sender),
    } = request
    else {
        panic!("missing request");
    };
    assert_eq!(payload["projectId"], 1);
    sender
        .send(Ok(json!({"tasks":[],"limited":false})))
        .unwrap();
    assert!(reply.try_recv().unwrap().is_ok());
    session.stop.store(true, Ordering::Relaxed);
    assert!(session.request(json!({"op":"tasks.list"})).is_err());
}
