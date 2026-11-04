#[allow(dead_code)]
#[path = "../src/terminal/host_apply.rs"]
mod host_apply;
#[allow(dead_code)]
#[path = "../src/terminal/host_events.rs"]
mod host_events;
#[allow(dead_code)]
#[path = "../src/terminal/host_history.rs"]
mod host_history;

use host_events::{Feed, State};
use std::sync::mpsc;
use tessel_host_client::terminal::{StreamRejection, TerminalEvent};
use tessel_host_protocol::pty::{Exit, Size};

#[test]
fn snapshot_resize_and_live_output_remain_ordered_across_frames() {
    let (sender, receiver) = mpsc::channel();
    let mut parser = vt100::Parser::new(3, 30, 10);
    parser.process(b"previous");
    sender
        .send(TerminalEvent::Snapshot {
            at: 1,
            size: Size { rows: 3, cols: 30 },
            alternate: false,
            resync: true,
            data: vec![b'x'; host_apply::FRAME_BYTES + 10],
        })
        .unwrap();
    sender
        .send(TerminalEvent::Resized {
            seq: 2,
            size: Size { rows: 4, cols: 40 },
        })
        .unwrap();
    sender.send(TerminalEvent::LeaseLost).unwrap();
    sender
        .send(TerminalEvent::Output {
            seq: 3,
            data: b"\x1b[Hlive\x1b[6n".to_vec(),
        })
        .unwrap();
    let mut feed = Feed::new(receiver, true);
    feed.poll(&mut parser, true, |_, _, _| {
        panic!("snapshot must not use live replies")
    });
    assert_eq!(feed.applied, None);
    assert_eq!(parser.screen().contents(), "previous");
    assert!(!feed.screen_ready());
    let mut live_calls = 0;
    feed.poll(&mut parser, true, |p, bytes, can_reply| {
        assert!(!can_reply);
        live_calls += 1;
        p.process(bytes);
    });
    assert_eq!(live_calls, 1);
    assert_eq!(feed.applied, Some(3));
    assert_eq!(parser.screen().size(), (4, 40));
    assert!(parser.screen().contents().starts_with("live"));
    assert!(feed.resynchronized);
    assert!(
        feed.history.is_empty(),
        "initial placeholder is not scrollback"
    );
    assert!(feed.screen_ready());
    assert!(!feed.lease);
}

#[test]
fn rejection_and_exit_preserve_the_last_screen_and_disable_input() {
    for ending in [
        TerminalEvent::StreamRejected(StreamRejection::OversizedSnapshot),
        TerminalEvent::Exit(Exit {
            seq: 2,
            code: Some(0),
            signal: None,
        }),
    ] {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(TerminalEvent::Output {
                seq: 1,
                data: b"retained".to_vec(),
            })
            .unwrap();
        sender.send(ending.clone()).unwrap();
        sender
            .send(TerminalEvent::Output {
                seq: 3,
                data: b"must not apply".to_vec(),
            })
            .unwrap();
        let mut feed = Feed::new(receiver, true);
        let mut parser = vt100::Parser::new(3, 40, 0);
        feed.poll(&mut parser, true, |p, bytes, _| p.process(bytes));
        feed.poll(&mut parser, true, |_, _, _| panic!("terminal stream ended"));
        assert_eq!(parser.screen().contents(), "retained");
        assert!(!feed.lease);
        assert!(!feed.screen_ready());
        assert!(matches!(
            feed.state,
            State::Rejected(_) | State::Exited { .. }
        ));
    }
}

#[test]
fn current_authority_overrides_stale_lease_and_replays_do_not_rewind_screen() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(TerminalEvent::Output {
            seq: 4,
            data: b"live".to_vec(),
        })
        .unwrap();
    sender
        .send(TerminalEvent::Output {
            seq: 3,
            data: b"old".to_vec(),
        })
        .unwrap();
    sender.send(TerminalEvent::Disconnected).unwrap();
    let mut feed = Feed::new(receiver, true);
    let mut parser = vt100::Parser::new(3, 30, 0);
    feed.poll(&mut parser, false, |p, bytes, can_reply| {
        assert!(!can_reply);
        p.process(bytes);
    });
    assert_eq!(feed.applied, Some(4));
    assert_eq!(parser.screen().contents(), "live");
    assert_eq!(feed.state, State::Disconnected);
    assert!(!feed.screen_ready());
}

#[test]
fn an_exit_cannot_rewind_the_applied_sequence() {
    for exit_seq in [3, 4, 5] {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(TerminalEvent::Output {
                seq: 4,
                data: b"retained".to_vec(),
            })
            .unwrap();
        sender
            .send(TerminalEvent::Exit(Exit {
                seq: exit_seq,
                code: Some(0),
                signal: None,
            }))
            .unwrap();
        let mut feed = Feed::new(receiver, true);
        let mut parser = vt100::Parser::new(3, 30, 0);
        feed.poll(&mut parser, true, |p, bytes, _| p.process(bytes));
        assert_eq!(parser.screen().contents(), "retained");
        assert_eq!(feed.applied, Some(exit_seq.max(4)));
        assert!(feed.needs_detach());
        if exit_seq < 4 {
            assert_eq!(
                feed.state,
                State::Rejected(host_events::Rejection::InvalidData)
            );
        } else {
            assert!(matches!(feed.state, State::Exited { .. }));
        }
    }
}

#[test]
fn invalid_local_events_drop_receiver_and_closed_channel_is_final() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(TerminalEvent::Output {
            seq: 1,
            data: vec![0; 65 * 1024],
        })
        .unwrap();
    let mut feed = Feed::new(receiver, true);
    let mut parser = vt100::Parser::new(3, 30, 0);
    feed.poll(&mut parser, true, |_, _, _| panic!("invalid output"));
    assert_eq!(
        feed.state,
        State::Rejected(host_events::Rejection::InvalidData)
    );
    assert!(feed.needs_detach());
    assert!(sender.send(TerminalEvent::LeaseLost).is_err());
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    let mut feed = Feed::new(receiver, true);
    feed.poll(&mut parser, true, |_, _, _| panic!("closed"));
    assert_eq!(feed.state, State::Closed);
    assert!(feed.needs_detach());
    feed.poll(&mut parser, true, |_, _, _| panic!("closed"));
}
