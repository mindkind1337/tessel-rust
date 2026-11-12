use super::*;
#[test]
fn pending_knock_departure_removes_prompt_without_joining_or_penalizing_invite() {
    let dir = tempfile::Builder::new()
        .prefix("tessel-knock-fixture-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let host = Host::start(
        dir.path(),
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    for dropping in [false, true] {
        let mut guest =
            GuestSession::connect(host.invitation.clone(), "Synthetic".into(), "a7".repeat(32));
        let deadline = Instant::now() + Duration::from_secs(5);
        let key = loop {
            if let Some(LiveShareAsk::Join { key, .. }) = host.snapshot().asks.first() {
                break key.clone();
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        };
        // Keep it pending across several liveness probes; an idle socket is not a disconnect.
        thread::sleep(Duration::from_millis(150));
        assert_eq!(host.snapshot().asks.len(), 1);
        if dropping {
            drop(guest);
        } else {
            guest.leave();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let snapshot = host.snapshot();
            if snapshot.asks.is_empty() && snapshot.guests.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "departed pending guest still appears on host"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // A stale approval is now a no-op, not a fabricated joined/offline guest or trust entry.
        host.answer_ask(AskAnswer::Join {
            key,
            allow: true,
            projects: vec![1],
            rights: ShareRights::collaborate(),
            trust: true,
        });
        let snapshot = host.snapshot();
        assert!(snapshot.guests.is_empty());
        assert!(snapshot.offline.is_empty());
        assert!(snapshot.trusted.is_empty());
        assert_eq!(snapshot.invites[0].uses, 0);
        assert_eq!(snapshot.invites[0].joined, 0);
        assert_eq!(snapshot.invites[0].failed_knocks, 0);
        assert!(!snapshot.invites[0].blocked);
    }
}
