use super::*;
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
