use super::*;

fn resumed_session(thread_id: &str) -> CodexAppServerSession {
    let (mut session, _) = CodexAppServerSession::start(config(Some(thread_id))).unwrap();
    assert!(matches!(
        session.receive(&json!({"id": 1, "result": {}})).unwrap(),
        CodexSessionIngress::Send(_)
    ));
    session
}

#[test]
fn a_resumed_thread_whose_history_is_gone_is_confirmed_then_reported_unavailable() {
    let mut session = resumed_session("01a0a16d-402c-79d3-bd49-e1128eb0d382");
    let confirmation = session
        .receive(&json!({"id": 2, "error": {"code": -32600, "message": "no rollout found for thread id 01a0a16d-402c-79d3-bd49-e1128eb0d382"}}))
        .unwrap();
    let CodexSessionIngress::Send(frames) = confirmation else {
        panic!("an unavailable-looking resume must be confirmed first: {confirmation:?}");
    };
    assert_eq!(
        decode(&frames[0]),
        json!({"id":3,"method":"thread/turns/list","params":{"threadId":"01a0a16d-402c-79d3-bd49-e1128eb0d382","limit":1}})
    );
    assert_eq!(
        session.receive(&json!({"id": 3, "error": {"code": -32600, "message": "invalid paginated history lineage for 01a0a16d-402c-79d3-bd49-e1128eb0d382: missing source rollout"}})),
        Err(CodexSessionError::ResumedThreadUnavailable)
    );
}

#[test]
fn a_resume_rejection_while_the_thread_history_still_exists_is_a_genuine_failure() {
    let mut session = resumed_session("thread-1");
    assert!(matches!(
        session
            .receive(&json!({"id": 2, "error": {"code": -32600, "message": "resume rejected"}}))
            .unwrap(),
        CodexSessionIngress::Send(_)
    ));
    assert_eq!(
        session.receive(&json!({"id": 3, "result": {"data": [], "nextCursor": null}})),
        Err(CodexSessionError::RequestRejected)
    );
}

#[test]
fn other_resume_errors_and_fresh_thread_errors_are_never_classified_as_unavailable() {
    let mut resumed = resumed_session("thread-1");
    assert_eq!(
        resumed.receive(&json!({"id": 2, "error": {"code": -32603, "message": "internal error"}})),
        Err(CodexSessionError::RequestRejected)
    );
    let (mut fresh, _) = CodexAppServerSession::start(config(None)).unwrap();
    assert!(matches!(
        fresh.receive(&json!({"id": 1, "result": {}})).unwrap(),
        CodexSessionIngress::Send(_)
    ));
    assert_eq!(
        fresh.receive(&json!({"id": 2, "error": {"code": -32600, "message": "invalid request"}})),
        Err(CodexSessionError::RequestRejected)
    );
}

fn ready_session() -> CodexAppServerSession {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    session.receive(&json!({"id": 1, "result": {}})).unwrap();
    session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    session
}

#[test]
fn the_gent_compact_command_starts_a_native_compaction_turn_in_either_announcement_order() {
    for announced_first in [false, true] {
        let mut session = ready_session();
        let frame = session
            .start_turn_after_interrupted_reply("/compact", &[], Some("cut off reply"))
            .unwrap();
        assert_eq!(
            decode(&frame),
            json!({"id":3,"method":"thread/compact/start","params":{"threadId":"thread-1"}})
        );
        assert!(!session.is_ready());
        let started = json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-c","status":"inProgress"}}});
        let response = json!({"id": 3, "result": {}});
        let (first, second) = if announced_first {
            (started, response)
        } else {
            (response, started)
        };
        session.receive(&first).unwrap();
        session.receive(&second).unwrap();
        assert_eq!(
            session
                .receive(&json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-c","status":"completed"}}}))
                .unwrap(),
            CodexSessionIngress::TurnEnded
        );
        assert!(session.is_ready());
    }
    let mut session = ready_session();
    let frame = session
        .start_turn_after_interrupted_reply("/compact please", &[], None)
        .unwrap();
    assert_eq!(decode(&frame)["method"], "turn/start");
}
