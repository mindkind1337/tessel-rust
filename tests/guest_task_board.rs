#![allow(dead_code)]
#[path = "../src/share/guest_rpc.rs"]
mod guest_rpc;
#[path = "../src/share/guest_text.rs"]
pub(crate) mod guest_text;
mod share {
    pub(crate) use crate::guest_rpc::{GuestRpcError, TASK_RESULT_NOT_AVAILABLE};
    pub(crate) use crate::guest_text;
}
#[path = "../src/guest_task_board.rs"]
mod guest_task_board;
use guest_task_board::*;
use serde_json::json;
use std::{sync::mpsc, time::Instant};
fn scope(session: u64, project: u64) -> Scope {
    Scope { session, project }
}
fn reply(title: &str) -> serde_json::Value {
    json!({"tasks":[{"id":"1","title":title,"column":"todo"}],"limited":false})
}
fn enqueue(
    board: &mut Controller,
    scope: Scope,
    action: Action,
    now: Instant,
) -> mpsc::SyncSender<Reply> {
    let (tx, rx) = mpsc::sync_channel(1);
    assert!(board.start(scope, action, now, |_| Ok(rx)));
    tx
}
#[test]
fn project_and_session_changes_discard_late_results_and_keep_project_drafts() {
    let mut board = Controller::default();
    let now = Instant::now();
    let a = scope(1, 1);
    let b = scope(1, 2);
    board.bind(Some(a), true);
    board.edit_draft(a, "Draft A".into());
    let old = enqueue(&mut board, a, Action::Refresh, now);
    board.bind(Some(b), true);
    assert!(old.send(Ok(reply("A"))).is_err());
    assert!(!board.poll(now));
    assert!(board.snapshot().cards.is_empty());
    board.edit_draft(b, "Draft B".into());
    board.bind(Some(a), true);
    assert_eq!(board.snapshot().draft, "Draft A");
    let old = enqueue(&mut board, a, Action::Refresh, now);
    board.bind(Some(scope(2, 1)), true);
    assert!(old.send(Ok(reply("Old session"))).is_err());
    assert!(board.snapshot().draft.is_empty());
    assert!(board.snapshot().cards.is_empty());
}
#[test]
fn matching_ack_clears_only_the_sent_draft_revision() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "First".into());
    let tx = enqueue(&mut board, s, Action::Add, now);
    board.edit_draft(s, "Second".into());
    tx.send(Ok(reply("First"))).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().draft, "Second");
    let tx = enqueue(&mut board, s, Action::Add, now);
    tx.send(Ok(reply("Second"))).unwrap();
    board.poll(now);
    assert!(board.snapshot().draft.is_empty());
}
#[test]
fn full_queue_pending_and_read_only_do_not_dispatch_mutations() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "Keep me".into());
    board.start(s, Action::Add, now, |_| Err("Full".into()));
    assert_eq!(board.snapshot().notice, Some(Notice::Busy));
    assert!(!board.busy());
    let _tx = enqueue(&mut board, s, Action::Refresh, now);
    assert!(!board.start(s, Action::Add, now, |_| panic!(
        "must not queue a second request"
    )));
    board.bind(Some(s), false);
    assert!(!board.edit_draft(s, "overwrite".into()));
    board.bind(None, false);
    board.bind(Some(s), false);
    board.start(s, Action::Add, now, |_| panic!("read-only mutation"));
    assert_eq!(board.snapshot().notice, Some(Notice::NotAllowed));
}
#[test]
fn mutation_timeout_reconciles_only_by_reading_and_keeps_uncertain_draft() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "Uncertain".into());
    let tx = enqueue(&mut board, s, Action::Add, now);
    assert!(!board.poll(now));
    assert!(board.poll(now + REQUEST_TIMEOUT));
    assert_eq!(board.snapshot().notice, Some(Notice::OutcomeUnknown));
    assert_eq!(board.snapshot().draft, "Uncertain");
    assert!(board.take_reconcile());
    assert!(!board.take_reconcile());
    assert!(tx.send(Ok(reply("Uncertain"))).is_err());
    let tx = enqueue(&mut board, s, Action::Refresh, now);
    tx.send(Ok(reply("Uncertain"))).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().notice, Some(Notice::Reconciled));
    assert_eq!(board.snapshot().draft, "Uncertain");
}
#[test]
fn revocation_hides_writes_but_accepts_visible_project_ack_and_disconnect_preserves_draft() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "First".into());
    let tx = enqueue(&mut board, s, Action::Add, now);
    board.bind(Some(s), false);
    tx.send(Ok(reply("First"))).unwrap();
    board.poll(now);
    assert!(!board.snapshot().writable);
    assert_eq!(board.snapshot().cards[0].title, "First");
    board.bind(Some(s), true);
    board.edit_draft(s, "Keep after disconnect".into());
    let _tx = enqueue(&mut board, s, Action::Add, now);
    board.bind(None, false);
    assert_eq!(board.snapshot().draft, "Keep after disconnect");
    assert_eq!(board.snapshot().notice, Some(Notice::OutcomeUnknown));
}
#[test]
fn malformed_host_replies_are_rejected_and_controls_cannot_reach_the_view() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    let bad = [
        json!({"tasks":[],"limited":"false"}),
        json!({"tasks":[{"id":"01","title":"x","column":"todo"}],"limited":false}),
        json!({"tasks":[{"id":"1","title":"x","column":"weird"}],"limited":false}),
        json!({"tasks":vec![json!({"id":"1","title":"x","column":"todo"});501],"limited":false}),
    ];
    for value in bad {
        let tx = enqueue(&mut board, s, Action::Refresh, now);
        tx.send(Ok(value)).unwrap();
        board.poll(now);
        assert_eq!(board.snapshot().notice, Some(Notice::InvalidReply));
    }
    let tx = enqueue(&mut board, s, Action::Refresh, now);
    tx.send(Ok(reply("\x1b[31mHello\u{202e}\nworld"))).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().cards[0].title, "Hello world");
    let tx = enqueue(&mut board, s, Action::Refresh, now);
    tx.send(Err("secret=synthetic C:/private/data\u{202e}".into()))
        .unwrap();
    board.poll(now);
    assert_eq!(
        board.snapshot().notice,
        Some(Notice::Host("secret=•••• …".into()))
    );
}
#[test]
fn host_failure_on_mutation_is_not_retried_and_stale_scope_cannot_send() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    assert!(!board.start(scope(2, 1), Action::Add, now, |_| panic!("stale action")));
    board.edit_draft(s, "Draft".into());
    let tx = enqueue(&mut board, s, Action::Add, now);
    tx.send(Err("transport closed".into())).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().notice, Some(Notice::OutcomeUnknown));
    assert!(board.take_reconcile());
    assert_eq!(board.snapshot().draft, "Draft");
}

#[test]
fn temporary_missing_scope_keeps_drafts_and_schedules_initial_read() {
    let mut board = Controller::default();
    let a = scope(1, 1);
    let b = scope(1, 2);
    board.bind(Some(a), true);
    assert!(board.take_reconcile());
    board.edit_draft(a, "Draft A".into());
    board.bind(None, false);
    board.bind(Some(b), true);
    assert!(board.take_reconcile());
    board.edit_draft(b, "Draft B".into());
    board.bind(Some(a), true);
    assert_eq!(board.snapshot().draft, "Draft A");
    board.bind(Some(scope(2, 1)), true);
    assert!(board.snapshot().draft.is_empty());
}
#[test]
fn oversized_native_edit_blocks_add_and_explicit_host_rejection_stays_explicit() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "Last valid text".into());
    board.touch_draft(s);
    board.start(s, Action::Add, now, |_| {
        panic!("must not send an older shorter draft")
    });
    assert_eq!(board.snapshot().notice, Some(Notice::InvalidInput));
    assert!(board.edit_draft(s, "Last valid text".into()));
    assert!(!board.snapshot().draft_over_limit);
    let tx = enqueue(&mut board, s, Action::Add, now);
    tx.send(Err(guest_rpc::GuestRpcError::Host(
        "No change was made.".into(),
    )))
    .unwrap();
    board.poll(now);
    assert_eq!(
        board.snapshot().notice,
        Some(Notice::Host("No change was made.".into()))
    );
    let tx = enqueue(&mut board, s, Action::Add, now);
    tx.send(Ok(reply("Last valid text"))).unwrap();
    board.poll(now);
    assert!(board.snapshot().notice.is_none());
    assert!(board.snapshot().draft.is_empty());
}
#[test]
fn control_only_title_does_not_hide_other_cards() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    let tx = enqueue(&mut board, s, Action::Refresh, now);
    tx.send(Ok(json!({"tasks":[{"id":"1","title":"\u{202e}","column":"todo"},{"id":"2","title":"Safe","column":"done"}],"limited":false}))).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().cards.len(), 1);
    assert_eq!(board.snapshot().cards[0].title, "Safe");
    assert!(board.snapshot().limited);
}

#[test]
fn host_pending_result_reconciles_without_repeating_mutation() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "Draft".into());
    let tx = enqueue(&mut board, s, Action::Add, now);
    assert!(!board.take_reconcile());
    tx.send(Err(guest_rpc::GuestRpcError::Host(
        share::TASK_RESULT_NOT_AVAILABLE.into(),
    )))
    .unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().notice, Some(Notice::OutcomeUnknown));
    assert!(board.take_reconcile());
    let tx = enqueue(&mut board, s, Action::Refresh, now);
    tx.send(Ok(reply("Draft"))).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().notice, Some(Notice::Reconciled));
    assert_eq!(board.snapshot().draft, "Draft");
    assert!(!board.take_reconcile());
}
#[test]
fn initial_read_survives_mutation_rejection_in_new_scope() {
    let mut board = Controller::default();
    let s = scope(1, 1);
    let now = Instant::now();
    board.bind(Some(s), true);
    board.edit_draft(s, "Draft".into());
    let tx = enqueue(&mut board, s, Action::Add, now);
    assert!(!board.take_reconcile());
    tx.send(Err(guest_rpc::GuestRpcError::Host("Quota reached".into())))
        .unwrap();
    board.poll(now);
    assert!(board.take_reconcile());
    let tx = enqueue(&mut board, s, Action::Refresh, now);
    tx.send(Ok(reply("Existing card"))).unwrap();
    board.poll(now);
    assert_eq!(board.snapshot().cards[0].title, "Existing card");
    assert!(!board.take_reconcile());
}
