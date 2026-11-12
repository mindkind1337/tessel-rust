use super::*;
use tessel_host_protocol::tree::Entry as WireEntry;

#[test]
fn worker_drains_more_than_stream_capacity_while_expand_waits_for_reply() {
    loopback_worker(false);
}

#[test]
fn worker_recovers_when_reconnection_exceeds_request_timeout() {
    loopback_worker(true);
}

fn loopback_worker(reconnect: bool) {
    use std::io::{self, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;
    use tessel_host_client::{Connector, Options, Pipe};
    use tessel_host_protocol::tree::{RootOpened, Subscribed, TreeLimits};
    use tessel_host_protocol::{
        Budgets, ClientFrame, Epoch, HelloOk, HostFrame, MAX_FRAME_BYTES, RawMessage, ReplyMessage,
        RootId, StreamId, StreamPos, VERSION, read_frame, write_frame,
    };

    struct SocketGuard(TcpStream);
    impl Drop for SocketGuard {
        fn drop(&mut self) {
            let _ = self.0.shutdown(Shutdown::Both);
        }
    }
    struct Local(std::net::SocketAddr);
    impl Connector for Local {
        fn connect(&self) -> io::Result<Pipe> {
            let socket = TcpStream::connect(self.0)?;
            socket.set_nodelay(true)?;
            Ok(Pipe {
                reader: Box::new(socket.try_clone()?),
                writer: Box::new(socket.try_clone()?),
                guard: Some(Box::new(SocketGuard(socket))),
            })
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let expansions = Arc::new(AtomicUsize::new(0));
    let counted = expansions.clone();
    let server = std::thread::spawn(move || {
        let mut interrupted = false;
        loop {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_nodelay(true).unwrap();
            if interrupted {
                // Keep the client reconnecting beyond one RPC's timeout.
                std::thread::sleep(Duration::from_millis(900));
            }
            let root = RootId::from_bytes([3; 16]);
            let stream = StreamId::from_bytes([4; 16]);
            let path = RelPath::from_components(vec![b"src".to_vec()]).unwrap();
            while let Ok(Some(frame)) = read_frame::<ClientFrame>(&mut socket, MAX_FRAME_BYTES) {
                let (Some(id), Some(message)) = (frame.request_id, frame.message) else {
                    continue;
                };
                let reply = match message.kind.as_str() {
                    "hello" => ReplyMessage::HelloOk(HelloOk {
                        version: VERSION,
                        capabilities: vec![tessel_host_protocol::Capability::Tree],
                        budgets: Budgets::SLOW_HOST,
                        host_epoch: Epoch::from_bytes([1; 16]),
                    })
                    .into_raw(),
                    "root.open" => {
                        assert_eq!(message.body["path"], "/project");
                        RawMessage::of(
                            "root.opened",
                            &RootOpened {
                                root: root.clone(),
                                label: "/display-only".into(),
                                limits: TreeLimits {
                                    max_watches: 32,
                                    exclusions: vec![],
                                },
                            },
                        )
                    }
                    "tree.subscribe" => RawMessage::of(
                        "tree.subscribed",
                        &Subscribed {
                            stream: stream.clone(),
                            revision: 0,
                        },
                    ),
                    "tree.expand" => {
                        let expand: tessel_host_protocol::tree::Expand = message.body_as().unwrap();
                        assert_eq!(expand.path, path);
                        let nth = counted.fetch_add(1, Ordering::SeqCst);
                        if reconnect && !interrupted {
                            interrupted = true;
                            let _ = socket.shutdown(Shutdown::Both);
                            break;
                        }
                        if nth == 0 {
                            // The reply is deliberately behind 600 events (> the 256-slot receiver).
                            for revision in 1..=600 {
                                let changed = Changed {
                                    revision,
                                    path: path.clone(),
                                    added: vec![],
                                    updated: vec![entry(b"data", EntryKind::File, revision)],
                                    removed: vec![],
                                };
                                let frame = HostFrame::event(
                                    StreamPos {
                                        id: stream.clone(),
                                        seq: revision,
                                    },
                                    RawMessage::of("tree.changed", &changed),
                                );
                                if write_frame(&mut socket, &frame, MAX_FRAME_BYTES).is_err() {
                                    return;
                                }
                            }
                        }
                        let revision = match nth {
                            0 => 0,
                            1 => 600,
                            _ => 601,
                        };
                        let mut listed =
                            listing(revision, vec![entry(b"data", EntryKind::File, revision)]);
                        listed.path = path.clone();
                        RawMessage::of("tree.listing", &listed)
                    }
                    _ => RawMessage::empty("ok"),
                };
                if write_frame(
                    &mut socket,
                    &HostFrame::reply_ok(id, reply),
                    MAX_FRAME_BYTES,
                )
                .and_then(|_| socket.flush())
                .is_err()
                {
                    return;
                }
                if message.kind == "tree.expand" && counted.load(Ordering::SeqCst) == 2 {
                    let changed = Changed {
                        revision: 601,
                        path: path.clone(),
                        added: vec![],
                        updated: vec![entry(b"data", EntryKind::File, 601)],
                        removed: vec![],
                    };
                    let frame = HostFrame::event(
                        StreamPos {
                            id: stream.clone(),
                            seq: 601,
                        },
                        RawMessage::of("tree.changed", &changed),
                    );
                    if write_frame(&mut socket, &frame, MAX_FRAME_BYTES).is_err() {
                        return;
                    }
                }
            }
            if !reconnect || !interrupted || counted.load(Ordering::SeqCst) > 1 {
                break;
            }
        }
    });
    let client = Client::connect(
        Box::new(Local(address)),
        Options {
            capabilities: vec!["tree"],
            request_timeout: if reconnect {
                Duration::from_millis(300)
            } else {
                Duration::from_secs(5)
            },
            ..Default::default()
        },
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mailbox = Arc::new(Mutex::new(None));
    let worker_mailbox = mailbox.clone();
    let worker_client = client.clone();
    let worker = std::thread::spawn(move || {
        run(
            worker_client,
            &Key {
                alias: "fixture".into(),
                root: "/project".into(),
                path: "/project/src".into(),
            },
            &worker_cancel,
            &worker_mailbox,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut observed = false;
    while Instant::now() < deadline {
        if let Some(Update::Listing(entries)) = mailbox.lock().unwrap().take()
            && entries
                .first()
                .is_some_and(|entry| entry.size == 601 && entry.path == "/project/src/data")
        {
            observed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cancel.store(true, Ordering::Release);
    client.close();
    worker.join().unwrap();
    server.join().unwrap();
    assert!(
        observed,
        "events must drain before the reply and the stale initial listing must be refreshed"
    );
    assert!((2..=4).contains(&expansions.load(Ordering::SeqCst)));
}

#[test]
fn stable_root_keeps_navigation_relative_and_refuses_prefix_escape() {
    assert!(relative("/project", "/project").unwrap().is_root());
    assert_eq!(
        relative("/project", "/project/src/nested")
            .unwrap()
            .to_bytes(),
        b"src/nested"
    );
    assert_eq!(relative("/", "/src").unwrap().to_bytes(), b"src");
    assert!(relative("/project", "/project-other/file").is_none());
    assert!(relative("/project", "/project/../escape").is_none());
    assert!(relative("/project", "/project//file").is_none());
    let path = relative("/project", "/project/src").unwrap();
    let mut cache = Cache {
        path: path.clone(),
        ..Default::default()
    };
    assert!(!cache.listing(listing(1, vec![])));
    let mut subdir = listing(2, vec![entry(b"x", EntryKind::File, 0)]);
    subdir.path = path;
    assert!(cache.listing(subdir));
    assert!(!cache.change(Changed {
        revision: 3,
        path: RelPath::root(),
        added: vec![],
        updated: vec![],
        removed: vec![]
    }));
}

fn entry(name: &[u8], kind: EntryKind, size: u64) -> WireEntry {
    WireEntry {
        name: Name::new(name.to_vec()).unwrap(),
        label: "untrusted display label".into(),
        kind,
        size,
        mtime_ms: 0,
        excluded: false,
    }
}
fn listing(revision: u64, entries: Vec<WireEntry>) -> Listing {
    Listing {
        path: RelPath::root(),
        revision,
        entries,
        watched: true,
        excluded: false,
        truncated: false,
    }
}
fn names(cache: &Cache) -> Vec<String> {
    match cache.snapshot("/project") {
        Update::Listing(entries) => entries.into_iter().map(|e| e.path).collect(),
        Update::Fallback => panic!("unexpected fallback"),
    }
}

#[test]
fn ordered_updates_ignore_old_revisions_and_use_raw_names() {
    let mut cache = Cache::default();
    assert!(cache.listing(listing(4, vec![entry(b"a.txt", EntryKind::File, 1)])));
    let change = |revision| Changed {
        revision,
        path: RelPath::root(),
        added: vec![entry(b"folder", EntryKind::Dir, 0)],
        updated: vec![entry(b"a.txt", EntryKind::File, 8)],
        removed: vec![],
    };
    assert!(!cache.change(change(4)));
    assert!(cache.change(change(6)));
    assert!(!cache.change(change(5)));
    assert_eq!(names(&cache), ["/project/folder", "/project/a.txt"]);
    let Update::Listing(entries) = cache.snapshot("/project") else {
        panic!()
    };
    assert_eq!(entries[1].size, 8);
    assert!(cache.change(Changed {
        revision: 7,
        path: RelPath::root(),
        added: vec![],
        updated: vec![],
        removed: vec![Name::new(b"a.txt".to_vec()).unwrap()]
    }));
    assert_eq!(names(&cache), ["/project/folder"]);
}

#[test]
fn unsupported_listings_and_non_utf8_never_become_lossy_paths() {
    let mut cache = Cache::default();
    let mut incomplete = listing(1, vec![]);
    incomplete.truncated = true;
    assert!(!cache.listing(incomplete));
    assert!(matches!(cache.snapshot("/project"), Update::Fallback));
    assert!(cache.listing(listing(2, vec![entry(&[0xff], EntryKind::File, 0)])));
    assert!(matches!(cache.snapshot("/project"), Update::Fallback));
    assert!(cache.listing(listing(
        3,
        vec![
            entry(b"link", EntryKind::Symlink, 0),
            entry(b"ok", EntryKind::File, 0)
        ]
    )));
    assert_eq!(names(&cache), ["/project/ok"]);
}

#[test]
fn reconnect_listing_replaces_old_epoch_and_deltas_without_base_are_ignored() {
    let mut cache = Cache::default();
    cache.listing(listing(99, vec![entry(b"old", EntryKind::File, 0)]));
    cache.entries = None;
    assert!(!cache.change(Changed {
        revision: 100,
        path: RelPath::root(),
        added: vec![],
        updated: vec![],
        removed: vec![]
    }));
    cache.listing(listing(1, vec![entry(b"new", EntryKind::File, 0)]));
    assert_eq!(names(&cache), ["/project/new"]);
    assert_eq!(cache.revision, 1);
}

#[test]
fn cancelling_a_worker_never_joins_it_on_the_ui_thread_or_applies_old_mailbox() {
    let cancel = Arc::new(AtomicBool::new(false));
    let mailbox = Arc::new(Mutex::new(Some(Update::Listing(vec![]))));
    let (release, waiting) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = waiting.recv();
    });
    let key = Key {
        alias: "fixture".into(),
        root: "/".into(),
        path: "/old".into(),
    };
    let mut watch = Watch {
        desired: Some(key.clone()),
        attempted: true,
        running: Some(Running {
            key,
            cancel: cancel.clone(),
            mailbox,
            thread,
            ended_reported: false,
        }),
    };
    assert!(watch.has_update());
    watch.invalidate();
    assert!(cancel.load(Ordering::Acquire));
    assert!(!watch.has_update());
    assert!(
        watch
            .poll(
                Some(Key {
                    alias: "fixture".into(),
                    root: "/".into(),
                    path: "/new".into()
                }),
                None
            )
            .is_none()
    );
    assert!(watch.running.is_some());
    release.send(()).unwrap();
    let running = watch.running.take().unwrap();
    running.thread.join().unwrap();
}

#[test]
fn mailbox_coalesces_to_the_latest_state() {
    let mailbox = Arc::new(Mutex::new(None));
    for _ in 0..1000 {
        publish(&mailbox, Update::Listing(vec![]));
    }
    publish(&mailbox, Update::Fallback);
    assert!(matches!(
        mailbox.lock().unwrap().take(),
        Some(Update::Fallback)
    ));
    assert!(mailbox.lock().unwrap().is_none());
}
