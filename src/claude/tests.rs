use super::*;
#[test]
fn native_review_claude_waits_without_consuming_draft_or_answering_permissions() {
    use crate::initial_turn::Phase;
    let mut c = Chat::new(1, "synthetic".into(), None);
    let rx = c.ready_sender_for_test();
    c.push("Claude", "Existing conversation");
    let epoch = c.review_identity().unwrap();
    c.gpui_set_composer("Owner draft".into());
    let draft = c.composer_state().0;
    assert_eq!(
        c.begin_review_turn("Review fixture", epoch).unwrap(),
        Phase::Pending
    );
    assert_eq!(c.composer_state(), (draft, "Owner draft"));
    c.gpui_set_composer(String::new());
    c.active = true;
    c.frame(approval("review-approval"));
    c.active = false;
    assert_eq!(
        c.begin_review_turn("Review fixture", epoch).unwrap(),
        Phase::Pending
    );
    assert_eq!(c.approvals.len(), 1);
    c.approvals.clear();
    c.active = true;
    c.frame(json!({"type":"control_request","request_id":"review-question","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Which?","header":"Fixture","options":[{"label":"A"},{"label":"B"}]}]}}}));
    c.active = false;
    assert!(c.questions.is_some());
    assert_eq!(
        c.begin_review_turn("Review fixture", epoch).unwrap(),
        Phase::Pending
    );
    c.questions = None;
    c.active = true;
    assert_eq!(
        c.begin_review_turn("Review fixture", epoch).unwrap(),
        Phase::Pending
    );
    c.active = false;
    assert!(rx.try_recv().is_err());
    assert_eq!(
        c.begin_review_turn("Review fixture", epoch).unwrap(),
        Phase::Submitted
    );
    let wire = rx.try_recv().unwrap();
    assert_eq!(wire["type"], "user");
    assert_eq!(wire["message"]["content"][0]["text"], "Review fixture");
    assert_eq!(c.transcript.back().unwrap().0, "Tessel");
    assert_eq!(c.composer_state(), (draft, ""));
    assert_eq!(
        c.begin_review_turn("Duplicate", epoch).unwrap(),
        Phase::Pending
    );
    assert!(rx.try_recv().is_err());
    c.stop();
    let _new_rx = c.ready_sender_for_test();
    assert_eq!(
        c.begin_review_turn("Stale", epoch).unwrap(),
        Phase::Cancelled
    );
}
#[test]
fn task_completion_claude_reads_text_blocks_but_never_thinking() {
    use crate::initial_turn::Native;
    use crate::task_completion::Outcome;
    for text in ["TASK_COMPLETE", "Not finished"] {
        let mut c = Chat::new(1, "synthetic".into(), None);
        let rx = c.ready_sender_for_test();
        c.begin_task_turn("Fixture", false).unwrap();
        rx.try_recv().unwrap();
        let token = c.task_completion.token();
        c.frame(json!({"type":"assistant","session_id":"synthetic-session","message":{"id":"answer","content":[{"type":"thinking","thinking":"TASK_COMPLETE"},{"type":"text","text":text}]}}));
        c.frame(json!({"type":"result","session_id":"synthetic-session","is_error":false,"subtype":"success"}));
        assert!(
            c.task_completion.outcome(token)
                == if text == "TASK_COMPLETE" {
                    Outcome::Complete
                } else {
                    Outcome::Cancelled
                }
        );
    }
}
#[test]
fn task_completion_claude_ignores_tools_deltas_foreign_sessions_and_interruptions() {
    use crate::initial_turn::Native;
    use crate::task_completion::Outcome;
    for case in 0..6 {
        let mut c = Chat::new(1, "synthetic".into(), None);
        let rx = c.ready_sender_for_test();
        c.begin_task_turn("Fixture", false).unwrap();
        rx.try_recv().unwrap();
        let token = c.task_completion.token();
        if case == 0 {
            c.frame(json!({"type":"stream_event","session_id":"synthetic-session","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"TASK_COMPLETE"}}}));
        } else {
            c.frame(json!({"type":if case == 1 {"user"} else {"assistant"},"session_id":if case == 2 {"foreign"} else {"synthetic-session"},"message":{"id":"answer","content":[{"type":if case == 3 {"tool_use"} else {"text"},"text":"TASK_COMPLETE"}]}}));
        }
        if case == 4 {
            c.gpui_interrupt();
        }
        if case == 5 {
            c.frame(json!({"type":"stream_event","session_id":"synthetic-session","event":{"type":"message_start","message":{"id":"next"}}}));
        }
        c.frame(json!({"type":"result","session_id":"synthetic-session","is_error":false,"subtype":"success"}));
        assert!(
            c.task_completion.outcome(token) == Outcome::Cancelled,
            "case {case}"
        );
    }
}
#[test]
fn native_task_first_turn_claude_preserves_draft_and_waits_for_permission() {
    use crate::initial_turn::{Native, Phase};
    let mut c = Chat::new(1, "synthetic".into(), None);
    let outgoing = c.ready_sender_for_test();
    c.gpui_set_composer("Unsent owner draft".into());
    let draft_id = c.composer_state().0;
    let prompt = "[Tessel task] Fixture\n\nExact synthetic instructions.";
    assert_eq!(c.begin_task_turn(prompt, false).unwrap(), Phase::Submitted);
    let frame = outgoing.try_recv().unwrap();
    assert_eq!(frame["type"], "user");
    assert_eq!(frame["session_id"], "synthetic-session");
    assert_eq!(frame["message"]["role"], "user");
    assert_eq!(frame["message"]["content"][0]["text"], prompt);
    assert!(valid_uuid(frame["uuid"].as_str().unwrap()));
    assert_eq!(c.composer_state(), (draft_id, "Unsent owner draft"));
    assert_eq!(c.begin_task_turn(prompt, true).unwrap(), Phase::Cancelled);
    c.frame(approval("task-write"));
    assert_eq!(c.approvals.len(), 1);
    assert!(
        outgoing.try_recv().is_err(),
        "no duplicate turn or automatic approval"
    );
}

#[test]
fn native_task_first_turn_claude_rejects_stale_accounts_resumes_and_transport_failures() {
    use crate::initial_turn::{Native, Phase};
    let mut c = Chat::new(1, "synthetic".into(), None);
    c.require_profile = true;
    let outgoing = c.ready_sender_for_test();
    assert_eq!(c.begin_task_turn("fixture", false).unwrap(), Phase::Pending);
    assert!(c.begin_task_turn("fixture", true).is_err());
    assert!(outgoing.try_recv().is_err());
    c.require_profile = false;
    c.resume = "10000000-0000-4000-8000-000000000001".into();
    assert_eq!(
        c.begin_task_turn("fixture", false).unwrap(),
        Phase::Cancelled
    );
    c.resume.clear();
    for prompt in [String::new(), "x".repeat(65537), "null\0fixture".into()] {
        assert!(c.begin_task_turn(&prompt, false).is_err());
    }
    assert!(outgoing.try_recv().is_err());
    c.gpui_set_composer("Keep this".into());
    drop(outgoing);
    assert!(c.begin_task_turn("fixture", false).is_err());
    assert!(!c.active);
    assert!(c.transcript.is_empty());
    assert_eq!(c.composer_state().1, "Keep this");
    c.ready = false;
    assert!(c.begin_task_turn("fixture", true).is_err());
    assert!(!c.busy(), "failed connection is not restarted");
}

#[test]
fn owner_message_preserves_whitespace_on_send_and_the_draft_on_failure() {
    let mut c = Chat::new(1, "synthetic".into(), None);
    let outgoing = c.ready_sender_for_test();
    c.gpui_set_composer("  Owner message\n".into());
    c.gpui_send_message();
    let frame = outgoing.try_recv().unwrap();
    assert_eq!(frame["message"]["content"][0]["text"], "  Owner message\n");
    assert_eq!(c.transcript.back().unwrap().1, "  Owner message\n");
    assert!(c.composer_state().1.is_empty());
    let mut failed = Chat::new(1, "synthetic".into(), None);
    let outgoing = failed.ready_sender_for_test();
    drop(outgoing);
    failed.gpui_set_composer("  Keep failed draft\n".into());
    let draft_id = failed.composer_state().0;
    failed.gpui_send_message();
    assert_eq!(failed.composer_state(), (draft_id, "  Keep failed draft\n"));
    assert!(failed.transcript.is_empty());
    assert!(!failed.active);
}

fn chat() -> Chat {
    let mut c = Chat::new(1, PathBuf::from("."), None);
    c.session = "10000000-0000-4000-8000-000000000001".into();
    c.active = true;
    c.ready = true;
    c
}
fn approval(id: &str) -> Value {
    json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"example.txt","content":"synthetic"}}})
}
#[test]
fn permissions_are_explicit_scoped_cancelled_and_refused_after_interrupt() {
    let mut c = chat();
    c.frame(approval("one"));
    assert_eq!(c.approvals.len(), 1);
    c.frame(approval("one"));
    assert_eq!(c.approvals.len(), 1);
    c.frame(json!({"type":"control_cancel_request","request_id":"one"}));
    assert!(c.approvals.is_empty());
    c.interrupting = true;
    c.frame(approval("two"));
    assert!(c.approvals.is_empty());
    c.interrupting = false;
    let mut other = approval("three");
    other["session_id"] = json!("other-session");
    c.frame(other);
    assert!(!c.active);
    assert!(c.approvals.is_empty());
}
#[test]
fn streaming_final_message_is_not_duplicated_and_ids_are_saved_only_after_response() {
    let mut c = chat();
    assert!(c.saved_session.is_none());
    c.frame(json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"message-1"}}}));
    c.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Bonjour"}}}));
    c.frame(json!({"type":"assistant","message":{"id":"message-1","content":[{"type":"text","text":"Bonjour Québec"}]}}));
    assert_eq!(c.transcript.len(), 1);
    assert_eq!(c.transcript[0].1, "Bonjour Québec");
    assert_eq!(c.saved_session.as_deref(), Some(c.session.as_str()));
    c.frame(json!({"type":"result","subtype":"success","is_error":false}));
    assert!(!c.active);
    assert!(c.approvals.is_empty());
    c.frame(json!({"type":"assistant","message":{"id":"stale","content":[{"type":"text","text":"stale"}]}}));
    assert_eq!(c.transcript.len(), 1);
}
#[test]
fn failed_turn_and_unsafe_permissions_never_look_successful() {
    let mut malformed = chat();
    malformed.frame(json!({"type":"result"}));
    assert_eq!(malformed.status, "Failed");
    let mut c = chat();
    c.frame(json!({"type":"result","subtype":"error_during_execution","is_error":true,"errors":["Synthetic failure"]}));
    assert_eq!(c.status, "Failed");
    assert!(!c.active);
    assert_eq!(c.error, "Synthetic failure");
    let mut c = chat();
    c.frame(json!({"type":"system","subtype":"init","permissionMode":"bypassPermissions"}));
    assert!(!c.ready);
    assert!(!c.error.is_empty());
    assert!(valid_uuid(&uuid().unwrap()));
    assert!(!valid_uuid("--resume"));
}
#[test]
fn metadata_roundtrips_without_connecting() {
    let mut state = crate::workspace::Workspace::default();
    let id = uuid().unwrap();
    state.claude_threads.insert(1, id.clone());
    let bytes = serde_json::to_vec(&state).unwrap();
    let restored: crate::workspace::Workspace = serde_json::from_slice(&bytes).unwrap();
    restored.validate().unwrap();
    let c = Chat::new(
        1,
        PathBuf::from("."),
        restored.claude_threads.get(&1).cloned(),
    );
    assert!(c.transport.is_none());
    assert_eq!(c.resume, id);
}

#[test]
fn questions_are_cancelled_and_cannot_arrive_after_turn_completion() {
    let mut c = chat();
    let request = json!({"type":"control_request","request_id":"question-1","request":{
        "subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{
            "question":"Which output?","header":"Output","options":[{"label":"PDF"},{"label":"HTML"}]
        }]}
    }});
    c.frame(request.clone());
    assert!(c.questions.is_some());
    assert!(c.approvals.is_empty());
    c.frame(json!({"type":"control_cancel_request","request_id":"question-1"}));
    assert!(c.questions.is_none());
    c.frame(request.clone());
    assert!(c.questions.is_none());
    c.seen.clear();
    c.frame(request.clone());
    assert!(c.questions.is_some());
    c.frame(json!({"type":"result","subtype":"success","is_error":false}));
    assert!(c.questions.is_none());
    c.frame(request);
    assert!(c.questions.is_none());
}

#[test]
fn model_draft_and_invalid_inherited_effort_are_explicit_without_connecting() {
    let mut chat = Chat::new(1, PathBuf::from("."), None);
    chat.options.model = "unfinished invalid".into();
    chat.commit_options();
    assert!(!chat.options_commit);
    assert!(!chat.error.is_empty());
    chat.options.model = "  model@20250929  ".into();
    chat.commit_options();
    assert!(chat.options_commit);
    assert!(chat.error.is_empty());
    assert_eq!(chat.options.model, "model@20250929");
    chat.options = Default::default();
    chat.defaults = Some(json!({"model":"inherited-model", "effort":"none"}));
    let error = chat.connect().unwrap_err();
    assert!(error.contains("inherited-model"));
    assert!(error.contains("none"));
    assert!(chat.transport.is_none());
}
