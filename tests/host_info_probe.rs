#[allow(dead_code)]
#[path = "../src/native_host_info.rs"]
mod native_host_info;

#[test]
fn workers_are_bounded_and_client_close_interrupts_silent_requests_before_their_deadline() {
    use std::io::{self, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tessel_host_client::{Client, Connector, Options, Pipe};
    use tessel_host_protocol::{
        Budgets, ClientFrame, Epoch, HelloOk, HostFrame, MAX_FRAME_BYTES, ReplyMessage, VERSION,
        read_frame, write_frame,
    };
    struct Stop(Option<mpsc::Sender<()>>);
    impl Drop for Stop {
        fn drop(&mut self) {
            if let Some(stop) = self.0.take() {
                let _ = stop.send(());
            }
        }
    }
    struct Silent(mpsc::Sender<()>);
    impl Connector for Silent {
        fn connect(&self) -> io::Result<Pipe> {
            let (reader, mut host_writer) = io::pipe()?;
            let (mut host_reader, writer) = io::pipe()?;
            let (stop, stopped) = mpsc::channel();
            let started = self.0.clone();
            std::thread::spawn(move || {
                while let Ok(Some(frame)) =
                    read_frame::<ClientFrame>(&mut host_reader, MAX_FRAME_BYTES)
                {
                    let (Some(id), Some(message)) = (frame.request_id, frame.message) else {
                        continue;
                    };
                    if message.kind == "hello" {
                        let reply = ReplyMessage::HelloOk(HelloOk {
                            version: VERSION,
                            capabilities: vec![],
                            budgets: Budgets::SLOW_HOST,
                            host_epoch: Epoch::from_bytes([1; 16]),
                        })
                        .into_raw();
                        if write_frame(
                            &mut host_writer,
                            &HostFrame::reply_ok(id, reply),
                            MAX_FRAME_BYTES,
                        )
                        .and_then(|_| host_writer.flush())
                        .is_err()
                        {
                            return;
                        }
                    } else if message.kind == "host.info" {
                        let _ = started.send(());
                        let _ = stopped.recv();
                        return; // Drop the pipe on guard cancellation, like the SSH child.
                    }
                }
            });
            Ok(Pipe {
                reader: Box::new(reader),
                writer: Box::new(writer),
                guard: Some(Box::new(Stop(Some(stop)))),
            })
        }
    }
    let (seen, started) = mpsc::channel();
    let client = Client::connect(
        Box::new(Silent(seen)),
        Options {
            capabilities: vec![],
            request_timeout: Duration::from_secs(30),
            ..Default::default()
        },
    )
    .unwrap();
    let cache = native_host_info::Cache::default();
    let workers: Vec<_> = (0..8)
        .map(|_| native_host_info::Worker::start(client.clone(), cache.clone()).unwrap())
        .collect();
    assert!(
        native_host_info::Worker::start(client.clone(), cache.clone()).is_none(),
        "global worker capacity must include blocked requests"
    );
    started
        .recv_timeout(Duration::from_secs(3))
        .expect("metadata request started");
    let closing = Instant::now();
    cache.close();
    for worker in &workers {
        worker.stop_sender().send(()).unwrap();
    }
    client.close();
    for worker in workers {
        worker.finish();
    }
    assert!(
        closing.elapsed() < Duration::from_secs(2),
        "close must interrupt the 30-second RPC deadline"
    );
    assert_eq!(
        cache
            .read(Some(&Epoch::from_bytes([1; 16])))
            .survives_logout,
        tessel_host_protocol::Survival::Unknown
    );
}
