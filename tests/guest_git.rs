#![allow(dead_code)]
#[path = "../src/share/guest_rpc.rs"]
mod guest_rpc;
#[path = "../src/share/guest_text.rs"]
pub(crate) mod guest_text;
mod share {
    pub(crate) use crate::guest_rpc::{GuestRpcError, TASK_RESULT_NOT_AVAILABLE};
    pub(crate) use crate::guest_text;
}
#[path = "../src/guest_git.rs"]
mod guest_git;
#[path = "../src/guest_task_board.rs"]
mod guest_task_board;
use guest_git::*;
use guest_task_board::Scope;
use serde_json::{Value, json};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
fn scope(project: u64) -> Scope {
    Scope {
        session: 1,
        project,
    }
}
fn start(c: &mut Controller, s: Scope, action: Action) -> mpsc::SyncSender<Reply> {
    let (tx, rx) = mpsc::sync_channel(1);
    assert!(c.start(s, action, Instant::now(), |v| {
        assert_eq!(v["projectId"], s.project);
        Ok(rx)
    }));
    tx
}
#[test]
fn late_git_status_cannot_cross_project_or_session() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    let tx = start(&mut c, scope(1), Action::Status);
    c.bind(Some(scope(2)), true);
    assert!(
        tx.send(Ok(json!({"status":"private old project"})))
            .is_err()
    );
    assert!(c.snapshot().status.is_none());
    let tx = start(&mut c, scope(2), Action::Status);
    c.bind(
        Some(Scope {
            session: 2,
            project: 2,
        }),
        true,
    );
    assert!(tx.send(Ok(json!({"status":"old session"}))).is_err());
    assert!(c.snapshot().status.is_none());
}
#[test]
fn commit_clears_only_matching_revision_after_true_ack() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    c.edit(scope(1), "first".into());
    let tx = start(&mut c, scope(1), Action::Commit);
    assert_eq!(c.snapshot().draft, "first");
    c.edit(scope(1), "newer".into());
    tx.send(Ok(json!({"ok":true}))).unwrap();
    c.poll(Instant::now());
    assert_eq!(c.snapshot().draft, "newer");
    let tx = start(&mut c, scope(1), Action::Commit);
    tx.send(Ok(json!({"ok":true}))).unwrap();
    c.poll(Instant::now());
    assert!(c.snapshot().draft.is_empty());
    assert_eq!(c.snapshot().notice, Some(Notice::Saved));
}
#[test]
fn commit_failure_or_malformed_ack_preserves_message_and_uncertainty() {
    for reply in [
        Ok(json!({"ok":false})),
        Ok(json!({"ok":"true"})),
        Err(guest_rpc::GuestRpcError::Transport("gone".into())),
        Err(guest_rpc::GuestRpcError::Host("git failed".into())),
    ] {
        let mut c = Controller::default();
        c.bind(Some(scope(1)), true);
        c.edit(scope(1), "keep".into());
        let tx = start(&mut c, scope(1), Action::Commit);
        tx.send(reply).unwrap();
        c.poll(Instant::now());
        assert_eq!(c.snapshot().draft, "keep");
        assert!(c.snapshot().uncertain);
        let tx = start(&mut c, scope(1), Action::Status);
        tx.send(Ok(json!({"status":""}))).unwrap();
        c.poll(Instant::now());
        assert!(
            c.snapshot().uncertain,
            "a clean status cannot prove that commit failed or succeeded"
        );
    }
}
#[test]
fn pending_commit_scope_change_keeps_origin_and_never_retries() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    c.edit(scope(1), "keep".into());
    let tx = start(&mut c, scope(1), Action::Commit);
    c.bind(Some(scope(2)), true);
    assert!(tx.send(Ok(json!({"ok":true}))).is_err());
    assert!(c.snapshot().uncertain);
    assert_eq!(c.snapshot().draft_scope, Some(scope(1)));
    assert!(
        !c.start(scope(2), Action::Commit, Instant::now(), |_| panic!(
            "wrong project"
        ))
    );
    assert!(!c.edit(scope(2), "overwrite".into()));
    c.bind(None, false);
    assert_eq!(c.snapshot().draft, "keep");
    c.bind(
        Some(Scope {
            session: 2,
            project: 1,
        }),
        true,
    );
    assert!(!c.snapshot().editable());
    assert!(c.discard());
    assert!(c.snapshot().editable());
}
#[test]
fn rights_revocation_blocks_mutations_and_pending_discard() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    c.edit(scope(1), "keep".into());
    let tx = start(&mut c, scope(1), Action::Commit);
    assert!(!c.discard());
    c.bind(Some(scope(1)), false);
    assert!(!c.edit(scope(1), "no".into()));
    tx.send(Err(guest_rpc::GuestRpcError::Host(
        "permission revoked".into(),
    )))
    .unwrap();
    c.poll(Instant::now());
    assert!(
        !c.start(scope(1), Action::Commit, Instant::now(), |_| panic!(
            "revoked"
        ))
    );
    assert_eq!(c.snapshot().draft, "keep");
}
#[test]
fn full_queue_and_timeout_return_without_consuming_draft() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    c.edit(scope(1), "keep".into());
    assert!(!c.start(scope(1), Action::Commit, Instant::now(), |_| Err(
        "full".into()
    )));
    assert!(!c.busy());
    assert_eq!(c.snapshot().notice, Some(Notice::Busy));
    let _tx = start(&mut c, scope(1), Action::Commit);
    assert!(
        !c.start(scope(1), Action::Status, Instant::now(), |_| panic!(
            "already pending"
        ))
    );
    c.poll(Instant::now() + Duration::from_secs(26));
    assert_eq!(c.snapshot().draft, "keep");
    assert!(c.snapshot().uncertain);
}
#[test]
fn oversized_or_invalid_messages_never_reach_host_and_are_not_truncated() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    for message in [" ".into(), "é".repeat(1001), "x\0y".into()] {
        c.edit(scope(1), message.clone());
        assert_eq!(c.snapshot().draft, message);
        assert!(
            !c.start(scope(1), Action::Commit, Instant::now(), |_| panic!(
                "invalid"
            ))
        );
    }
}
#[test]
fn status_requires_bounded_string_and_distinguishes_empty_repository() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    for value in [
        json!({"status":false}),
        json!({"status":"x".repeat(256*1024+1)}),
    ] {
        let tx = start(&mut c, scope(1), Action::Status);
        tx.send(Ok(value)).unwrap();
        c.poll(Instant::now());
        assert_eq!(c.snapshot().notice, Some(Notice::Invalid));
        assert!(c.snapshot().status.is_none());
    }
    let tx = start(&mut c, scope(1), Action::Status);
    tx.send(Ok(json!({"status":""}))).unwrap();
    c.poll(Instant::now());
    assert_eq!(c.snapshot().status, Some(String::new()));
}
#[test]
fn commit_payload_is_trimmed_and_contains_exact_project() {
    let mut c = Controller::default();
    c.bind(Some(scope(3)), true);
    c.edit(scope(3), "  message  ".into());
    let (_tx, rx) = mpsc::sync_channel(1);
    c.start(scope(3), Action::Commit, Instant::now(), |v: Value| {
        assert_eq!(
            v,
            json!({"op":"scm.commit","projectId":3,"message":"message"})
        );
        Ok(rx)
    });
    assert_eq!(c.snapshot().draft, "  message  ");
}

#[test]
fn returning_to_draft_project_restores_editing_without_wrong_project_notice() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    c.edit(scope(1), "draft".into());
    c.bind(Some(scope(2)), true);
    assert!(!c.snapshot().editable());
    c.bind(Some(scope(1)), true);
    assert!(c.snapshot().editable());
    assert_eq!(c.snapshot().notice, None);
    assert_eq!(c.snapshot().draft, "draft");
}

#[test]
fn hidden_and_partial_status_remain_explicit() {
    let mut c = Controller::default();
    c.bind(Some(scope(1)), true);
    let tx = start(&mut c, scope(1), Action::Status);
    tx.send(Ok(json!({"status":"","hidden":2,"truncated":true})))
        .unwrap();
    c.poll(Instant::now());
    assert_eq!(c.snapshot().hidden, 2);
    assert!(c.snapshot().limited);
    c.bind(Some(scope(2)), true);
    assert_eq!(c.snapshot().hidden, 0);
    assert!(!c.snapshot().limited);
}
