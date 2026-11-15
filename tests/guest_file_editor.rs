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
#[path = "../src/guest_task_board.rs"]
mod guest_task_board;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use guest_file_editor::*;
use guest_task_board::Scope;
use serde_json::json;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
fn key(project: u64, path: &str) -> Key {
    Key {
        scope: Scope {
            session: 1,
            project,
        },
        path: path.into(),
    }
}
fn read(
    editor: &mut Editor,
    key: Key,
    now: Instant,
) -> mpsc::SyncSender<Result<serde_json::Value, guest_rpc::GuestRpcError>> {
    let (tx, rx) = mpsc::sync_channel(1);
    let expected = key.clone();
    assert!(editor.open(key, now, |payload| {
        assert_eq!(payload["projectId"], expected.scope.project);
        assert_eq!(payload["path"], expected.path);
        Ok(rx)
    }));
    tx
}
fn loaded(editor: &mut Editor, key: Key, text: &str, now: Instant) {
    editor.bind(Some(key.scope), true);
    read(editor, key, now)
        .send(Ok(json!({"data":B64.encode(text)})))
        .unwrap();
    editor.poll(now);
}
#[test]
fn files_read_is_scoped_and_late_reply_cannot_load_a_different_project() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "same.txt");
    let b = key(2, "same.txt");
    editor.bind(Some(a.scope), true);
    let late = read(&mut editor, a, now);
    editor.bind(Some(b.scope), true);
    let current = read(&mut editor, b.clone(), now);
    assert!(late.send(Ok(json!({"data":B64.encode("A")}))).is_err());
    current.send(Ok(json!({"data":B64.encode("B")}))).unwrap();
    editor.poll(now);
    assert_eq!(editor.document.as_ref().unwrap().key, b);
    assert_eq!(editor.document.as_ref().unwrap().text.as_str(), "B");
}
#[test]
fn opening_another_file_clears_loaded_text_and_unsaved_drafts_require_explicit_discard() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "a.txt");
    let b = key(1, "b.txt");
    loaded(&mut editor, a, "A", now);
    let tx = read(&mut editor, b.clone(), now);
    assert!(editor.document.is_none());
    tx.send(Ok(json!({"data":B64.encode("B")}))).unwrap();
    editor.poll(now);
    editor.edit(&b, "Edited B".into());
    editor.open(key(1, "c.txt"), now, |_| {
        panic!("unsaved draft must not be overwritten")
    });
    assert_eq!(editor.notice, Some(Notice::UnsavedDraft));
    assert_eq!(editor.document.as_ref().unwrap().text.as_str(), "Edited B");
    assert!(editor.discard());
    assert!(editor.document.is_none());
}
#[test]
fn project_change_preserves_dirty_text_but_blocks_saving_to_new_project() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "a.txt");
    loaded(&mut editor, a.clone(), "A", now);
    editor.edit(&a, "Draft".into());
    editor.bind(Some(key(2, "a.txt").scope), true);
    assert!(!editor.editable());
    editor.save(&a, now, |_| panic!("wrong project"));
    assert_eq!(editor.document.as_ref().unwrap().text.as_str(), "Draft");
    editor.bind(Some(a.scope), true);
    assert!(editor.editable());
}
#[test]
fn save_ack_keeps_newer_edits_and_retains_exact_target() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(7, "a.txt");
    loaded(&mut editor, a.clone(), "A", now);
    editor.edit(&a, "Sent".into());
    let (tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |payload| {
        assert_eq!(payload["projectId"], 7);
        assert_eq!(payload["path"], "a.txt");
        assert_eq!(payload["data"], B64.encode("Sent"));
        Ok(rx)
    });
    editor.edit(&a, "Newer".into());
    tx.send(Ok(json!({"ok":true}))).unwrap();
    editor.poll(now);
    assert_eq!(editor.document.as_ref().unwrap().text.as_str(), "Newer");
    assert!(editor.document.as_ref().unwrap().dirty());
}
#[test]
fn timeout_or_invalid_save_ack_preserves_draft_without_retry() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "a.txt");
    loaded(&mut editor, a.clone(), "A", now);
    editor.edit(&a, "Sent".into());
    let (_tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |_| Ok(rx));
    editor.poll(now + Duration::from_secs(21));
    assert_eq!(editor.notice, Some(Notice::Unknown));
    assert!(editor.document.as_ref().unwrap().dirty());
    assert!(!editor.busy());
    let (tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |_| Ok(rx));
    tx.send(Ok(json!({"ok":"true"}))).unwrap();
    editor.poll(now);
    assert_eq!(editor.notice, Some(Notice::Unknown));
}
#[test]
fn unsafe_paths_binary_reads_and_oversize_writes_fail_closed() {
    let now = Instant::now();
    let mut editor = Editor::default();
    editor.bind(Some(key(1, "a").scope), true);
    for path in ["/private", "../a", "a/../b", "C:/secret", "a\\b", ""] {
        editor.open(key(1, path), now, |_| panic!("invalid path sent"));
        assert_eq!(editor.notice, Some(Notice::Invalid));
    }
    let a = key(1, "a.txt");
    read(&mut editor, a.clone(), now)
        .send(Ok(json!({"data":B64.encode([255])})))
        .unwrap();
    editor.poll(now);
    assert!(editor.document.is_none());
    loaded(&mut editor, a.clone(), "A", now);
    editor.edit(&a, "x".repeat(MAX_EDIT + 1));
    editor.save(&a, now, |_| panic!("oversized write sent"));
    assert_eq!(editor.notice, Some(Notice::TooLarge));
    assert_eq!(editor.document.as_ref().unwrap().text.len(), MAX_EDIT + 1);
    assert!(editor.editable());
    editor.edit(&a, "Short again".into());
    let (tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |_| Ok(rx));
    tx.send(Ok(json!({"ok":true}))).unwrap();
    editor.poll(now);
    assert!(!editor.document.as_ref().unwrap().dirty());
}

#[test]
fn reverting_text_during_pending_save_still_retains_uncertain_draft_on_scope_change() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "a.txt");
    loaded(&mut editor, a.clone(), "Original", now);
    editor.edit(&a, "Sent".into());
    let (_tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |_| Ok(rx));
    editor.edit(&a, "Original".into());
    assert!(editor.has_draft());
    editor.bind(Some(key(2, "a.txt").scope), true);
    assert!(editor.document.is_some());
    assert!(editor.has_draft());
    assert_eq!(editor.notice, Some(Notice::Unknown));
    editor.bind(Some(a.scope), true);
    editor.open(a.clone(), now, |_| {
        panic!("uncertain draft must not be silently replaced")
    });
    assert_eq!(editor.notice, Some(Notice::Unknown));
    let (tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |_| Ok(rx));
    tx.send(Ok(json!({"ok":true}))).unwrap();
    editor.poll(now);
    assert!(!editor.has_draft());
}

#[test]
fn revoked_rights_and_host_read_only_keep_text_but_never_save() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "a.txt");
    loaded(&mut editor, a.clone(), "Original", now);
    editor.edit(&a, "Keep me".into());
    editor.bind(Some(a.scope), false);
    assert!(!editor.editable());
    editor.save(&a, now, |_| panic!("rights revoked"));
    assert_eq!(editor.document.as_ref().unwrap().text.as_str(), "Keep me");
    editor.bind(None, false);
    assert_eq!(editor.document.as_ref().unwrap().text.as_str(), "Keep me");
    editor.discard();
    editor.bind(Some(a.scope), true);
    read(&mut editor, a.clone(), now)
        .send(Ok(json!({"data":B64.encode("Read only"),"readOnly":true})))
        .unwrap();
    editor.poll(now);
    assert!(!editor.editable());
    assert!(!editor.edit(&a, "Change".into()));
    editor.save(&a, now, |_| panic!("host marked read only"));
}
#[test]
fn maximum_edit_and_path_fit_the_existing_request_frame() {
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, &"q".repeat(4096));
    loaded(&mut editor, a.clone(), "Original", now);
    editor.edit(&a, "x".repeat(MAX_EDIT));
    let (_tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |payload| {
        assert!(serde_json::to_vec(&payload).unwrap().len() <= 256 * 1024);
        Ok(rx)
    });
    assert!(editor.busy());
}

#[test]
fn readonly_preview_is_utf8_bounded_without_truncating_document() {
    let mut editor = Editor::default();
    let text = "€".repeat(100_000);
    loaded(&mut editor, key(1, "large.txt"), &text, Instant::now());
    let document = editor.document.as_ref().unwrap();
    let (preview, partial) = document.preview();
    assert!(partial);
    assert_eq!(preview.len(), 65535);
    assert!(text.starts_with(preview));
    assert_eq!(document.text.as_str(), text);
}

#[test]
fn conflict_preserves_draft_and_saves_use_the_acknowledged_baseline() {
    use sha2::{Digest, Sha256};
    let hash = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
    let now = Instant::now();
    let mut editor = Editor::default();
    let a = key(1, "a.txt");
    loaded(&mut editor, a.clone(), "\u{feff}original\r\n", now);
    editor.edit(&a, "first draft".into());
    let (tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |p| {
        assert_eq!(p["expectHash"], hash("\u{feff}original\r\n"));
        Ok(rx)
    });
    editor.edit(&a, "later draft".into());
    tx.send(Ok(json!({"ok":true}))).unwrap();
    editor.poll(now);
    let (tx, rx) = mpsc::sync_channel(1);
    editor.save(&a, now, |p| {
        assert_eq!(p["expectHash"], hash("first draft"));
        Ok(rx)
    });
    tx.send(Err(guest_rpc::GuestRpcError::Host(
        "File conflict: host changed".into(),
    )))
    .unwrap();
    editor.poll(now);
    assert_eq!(editor.notice, Some(Notice::Conflict));
    assert_eq!(
        editor.document.as_ref().unwrap().text.as_str(),
        "later draft"
    );
    assert!(editor.document.as_ref().unwrap().dirty());
    editor.open(key(1, "other.txt"), now, |_| {
        panic!("must keep conflicted draft")
    });
    assert_eq!(editor.notice, Some(Notice::UnsavedDraft));
}
