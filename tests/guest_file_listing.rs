#![allow(dead_code)]
#[path = "../src/share/guest_rpc.rs"]
mod guest_rpc;
#[path = "../src/share/guest_text.rs"]
pub(crate) mod guest_text;
mod share {
    pub(crate) use crate::guest_rpc::{GuestRpcError, TASK_RESULT_NOT_AVAILABLE};
    pub(crate) use crate::guest_text;
}
#[path = "../src/guest_file_editor.rs"]
mod guest_file_editor;
#[path = "../src/guest_file_listing.rs"]
mod guest_file_listing;
#[path = "../src/guest_task_board.rs"]
mod guest_task_board;
use guest_file_listing::*;
use guest_task_board::Scope;
use serde_json::json;
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
#[test]
fn late_directory_replies_cannot_cross_project_scope() {
    let now = Instant::now();
    let mut listing = Listing::default();
    listing.bind(Some(scope(1)));
    assert!(listing.take_initial());
    let (old, rx) = mpsc::sync_channel(1);
    listing.start(scope(1), "src".into(), now, |v| {
        assert_eq!(v["projectId"], 1);
        Ok(rx)
    });
    listing.bind(Some(scope(2)));
    assert!(listing.entries.is_empty());
    assert!(listing.path.is_empty());
    assert!(old.send(Ok(json!({"entries":[]}))).is_err());
    let (tx, rx) = mpsc::sync_channel(1);
    listing.start(scope(2), "".into(), now, |v| {
        assert_eq!(v["projectId"], 2);
        Ok(rx)
    });
    tx.send(Ok(
        json!({"entries":[{"name":"b.txt","path":"b.txt","directory":false}]}),
    ))
    .unwrap();
    listing.poll(now);
    assert_eq!(listing.entries[0].name, "b.txt");
}
#[test]
fn listing_rejects_bad_paths_duplicates_and_wrong_parent() {
    let now = Instant::now();
    let mut listing = Listing::default();
    listing.bind(Some(scope(1)));
    for rows in [
        json!([{"name":"a","path":"../a","directory":false}]),
        json!([{"name":"a","path":"other/a","directory":false}]),
        json!([{"name":"a","path":"src/a","directory":"false"}]),
        json!([{"name":"a","path":"src/a","directory":false},{"name":"a","path":"src/a","directory":false}]),
    ] {
        let (tx, rx) = mpsc::sync_channel(1);
        listing.start(scope(1), "src".into(), now, |_| Ok(rx));
        tx.send(Ok(json!({"entries":rows}))).unwrap();
        listing.poll(now);
        assert_eq!(listing.notice, Some(Notice::Invalid));
        assert!(listing.entries.is_empty());
    }
}
#[test]
fn pending_timeout_and_full_queue_are_explicit_and_never_block() {
    let now = Instant::now();
    let mut listing = Listing::default();
    listing.bind(Some(scope(1)));
    listing.take_initial();
    listing.start(scope(1), "".into(), now, |_| Err("full".into()));
    assert_eq!(listing.notice, Some(Notice::Busy));
    assert!(!listing.take_initial());
    let (_tx, rx) = mpsc::sync_channel(1);
    listing.start(scope(1), "".into(), now, |_| Ok(rx));
    assert!(!listing.start(scope(1), "".into(), now, |_| panic!("already pending")));
    listing.poll(now + Duration::from_secs(21));
    assert_eq!(listing.notice, Some(Notice::Unavailable));
    assert!(!listing.busy());
}

#[test]
fn unsupported_names_leave_valid_entries_available_with_partial_notice() {
    let now = Instant::now();
    let mut listing = Listing::default();
    listing.bind(Some(scope(1)));
    let (tx, rx) = mpsc::sync_channel(1);
    listing.start(scope(1), "".into(), now, |_| Ok(rx));
    tx.send(Ok(json!({"entries":[
        {"name":"odd:name","path":"odd:name","directory":false},
        {"name":"ok.txt","path":"ok.txt","directory":false}
    ]})))
    .unwrap();
    listing.poll(now);
    assert_eq!(listing.notice, Some(Notice::Partial));
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].path, "ok.txt");
}
