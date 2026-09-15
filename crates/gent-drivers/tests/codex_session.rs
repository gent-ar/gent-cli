use gent_drivers::codex_session::{
    CodexAppServerSession, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
    CodexTurnOptions,
};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};
fn config(resume_thread_id: Option<&str>) -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: resume_thread_id.map(str::to_owned),
        turn_options: options(AgentChatMode::Agent),
        mcp_servers: None,
    }
}

fn options(mode: AgentChatMode) -> CodexTurnOptions {
    CodexTurnOptions::from_selection(
        &AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Medium,
            mode,
        },
        Some("/work"),
    )
    .unwrap()
}

fn decode(frame: &[u8]) -> Value {
    assert_eq!(frame.last(), Some(&b'\n'));
    let mut decoded: Value = serde_json::from_slice(&frame[..frame.len() - 1]).unwrap();
    assert_eq!(decoded["jsonrpc"], json!("2.0"));
    decoded.as_object_mut().unwrap().remove("jsonrpc");
    decoded
}

#[test]
fn initialize_then_start_then_turn_uses_only_correlated_responses() {
    let (mut session, initialize) = CodexAppServerSession::start(config(None)).unwrap();
    assert_eq!(
        decode(&initialize),
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"gent","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"requestAttestation":false}}})
    );
    let frames = match session.receive(&json!({"id": 1, "result": {}})).unwrap() {
        CodexSessionIngress::Send(frames) => frames,
        other => panic!("unexpected ingress: {other:?}"),
    };
    assert_eq!(
        decode(&frames[0]),
        json!({"method":"initialized","params":{}})
    );
    assert_eq!(
        decode(&frames[1]),
        json!({"id":2,"method":"thread/start","params":{"cwd":"/work"}})
    );
    assert_eq!(
        session.receive(&json!({"id": 9, "result": {"thread": {"id": "other"}}})),
        Err(CodexSessionError::UncorrelatedResponse)
    );
    assert_eq!(
        session
            .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
            .unwrap(),
        CodexSessionIngress::Ready {
            thread_id: "thread-1".into()
        }
    );
    let turn = session.start_turn("hello").unwrap();
    assert_eq!(
        decode(&turn),
        json!({"id":3,"method":"turn/start","params":{"threadId":"thread-1","input":[{"type":"text","text":"hello"}],"model":"gpt-5.6","effort":"medium","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
    );
}

#[test]
fn resume_requires_the_exact_recorded_thread_and_never_exports_it() {
    let (mut session, _) = CodexAppServerSession::start(config(Some("thread-1"))).unwrap();
    let frames = match session.receive(&json!({"id": 1, "result": {}})).unwrap() {
        CodexSessionIngress::Send(frames) => frames,
        other => panic!("unexpected ingress: {other:?}"),
    };
    assert_eq!(
        decode(&frames[1]),
        json!({"id":2,"method":"thread/resume","params":{"cwd":"/work","threadId":"thread-1","excludeTurns":true}})
    );
    assert_eq!(
        session.receive(&json!({"id": 2, "result": {"thread": {"id": "thread-2"}}})),
        Err(CodexSessionError::ResumedThreadMismatch)
    );
    assert!(!session.is_ready());
}

#[test]
fn turn_notification_and_response_must_agree_before_a_turn_becomes_live() {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    let _ = session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let _ = session.start_turn("hello").unwrap();
    assert_eq!(
        session.receive(&json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}})).unwrap(),
        CodexSessionIngress::Ignored
    );
    assert_eq!(
        session.receive(&json!({"id":3,"result":{"turn":{"id":"turn-2"}}})),
        Err(CodexSessionError::TurnIdentityMismatch)
    );
}

#[test]
fn only_the_matching_terminal_notification_releases_the_next_turn() {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    let _ = session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let _ = session.start_turn("hello").unwrap();
    let _ = session
        .receive(&json!({"id":3,"result":{"turn":{"id":"turn-1"}}}))
        .unwrap();
    assert_eq!(
        session.start_turn("again"),
        Err(CodexSessionError::TurnAlreadyActive)
    );
    assert_eq!(
        session.receive(&json!({"method":"turn/completed","params":{"threadId":"other","turn":{"id":"turn-1"}}})).unwrap(),
        CodexSessionIngress::Ignored
    );
    assert_eq!(
        session.receive(&json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}})).unwrap(),
        CodexSessionIngress::TurnEnded
    );
    assert_eq!(
        decode(&session.start_turn("again").unwrap())["id"],
        json!(4)
    );
}

#[test]
fn empty_terminal_notification_releases_the_owned_turn() {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    let _ = session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let _ = session.start_turn("hello").unwrap();
    let _ = session
        .receive(&json!({"id": 3, "result": {"turn": {"id": "turn-1"}}}))
        .unwrap();
    assert_eq!(
        session
            .receive(&json!({"method": "turn/completed", "params": {}}))
            .unwrap(),
        CodexSessionIngress::TurnEnded
    );
    assert!(session.is_ready());
}

#[test]
fn empty_terminal_notification_can_follow_the_started_notification() {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    let _ = session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let _ = session.start_turn("hello").unwrap();
    let _ = session
        .receive(&json!({"method": "turn/started", "params": {"threadId": "thread-1", "turn": {"id": "turn-1"}}}))
        .unwrap();
    assert_eq!(
        session
            .receive(&json!({"method": "turn/completed", "params": {}}))
            .unwrap(),
        CodexSessionIngress::TurnEnded
    );
}

#[test]
fn live_turn_interrupt_uses_json_rpc_without_destroying_the_session() {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    let _ = session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let _ = session.start_turn("hello").unwrap();
    let _ = session
        .receive(&json!({"id": 3, "result": {"turn": {"id": "turn-1"}}}))
        .unwrap();
    assert_eq!(
        decode(&session.interrupt().unwrap()),
        json!({"id":4,"method":"turn/interrupt","params":{"threadId":"thread-1","turnId":"turn-1"}})
    );
    assert_eq!(
        session.interrupt(),
        Err(CodexSessionError::InterruptAlreadyRequested)
    );
    assert_eq!(
        session.receive(&json!({"id": 4, "result": {}})).unwrap(),
        CodexSessionIngress::Ignored
    );
    assert_eq!(
        session.receive(&json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}})).unwrap(),
        CodexSessionIngress::TurnEnded
    );
    assert!(session.is_ready());
}

#[test]
fn invalid_inputs_and_failed_responses_are_bounded_and_secret_free() {
    assert_eq!(
        CodexAppServerSession::start(CodexSessionConfig {
            working_directory: Some(String::new()),
            resume_thread_id: None,
            turn_options: options(AgentChatMode::Ask),
            mcp_servers: None,
        }),
        Err(CodexSessionError::InvalidWorkingDirectory)
    );
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    assert_eq!(
        session.start_turn(""),
        Err(CodexSessionError::InvalidPrompt)
    );
    assert_eq!(
        session.receive(&json!({"id": 1, "error": {"message": "do not surface this"}})),
        Err(CodexSessionError::RequestRejected)
    );
    assert_eq!(
        session.start_turn("hello"),
        Err(CodexSessionError::SessionFailed)
    );
    let (mut fresh, _) = CodexAppServerSession::start(config(None)).unwrap();
    assert_eq!(
        fresh.receive(&json!({"id": 9, "error": {"message": "also do not surface this"}})),
        Err(CodexSessionError::UncorrelatedResponse)
    );
    assert!(matches!(
        fresh.receive(&json!({"id": 1, "result": {}})).unwrap(),
        CodexSessionIngress::Send(_)
    ));
}

#[test]
fn malformed_or_out_of_order_frames_leave_the_handshake_bounded() {
    let (mut session, _) = CodexAppServerSession::start(config(None)).unwrap();
    assert_eq!(
        session.start_turn("too early"),
        Err(CodexSessionError::ThreadNotReady)
    );
    assert_eq!(
        session.receive(&json!({})),
        Err(CodexSessionError::MalformedResponse)
    );
    assert_eq!(
        session.receive(&json!({"id": 1})),
        Err(CodexSessionError::MalformedResponse)
    );
    assert_eq!(
        session.receive(&json!({"method": "turn/started"})).unwrap(),
        CodexSessionIngress::Ignored
    );
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    assert_eq!(
        session.receive(&json!({"id": 2, "result": {"thread": {"id": ""}}})),
        Err(CodexSessionError::MalformedResponse)
    );
}

#[test]
fn native_connection_fields_reject_empty_and_unbounded_values() {
    for resume_thread_id in [Some(String::new()), Some("x".repeat(513))] {
        assert_eq!(
            CodexAppServerSession::start(CodexSessionConfig {
                working_directory: None,
                resume_thread_id,
                turn_options: options(AgentChatMode::Ask),
                mcp_servers: None,
            }),
            Err(CodexSessionError::InvalidThreadId)
        );
    }
    assert_eq!(
        CodexAppServerSession::start(CodexSessionConfig {
            working_directory: Some("x".repeat(4_097)),
            resume_thread_id: None,
            turn_options: options(AgentChatMode::Ask),
            mcp_servers: None,
        }),
        Err(CodexSessionError::InvalidWorkingDirectory)
    );
}

#[test]
fn selection_options_reject_other_providers_and_preserve_safe_sandbox_modes() {
    assert_eq!(
        CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "claude".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
            None,
        ),
        Err(CodexSessionError::UnsupportedSelection)
    );
    assert_eq!(
        CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: " \t".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
            None,
        ),
        Err(CodexSessionError::InvalidModel)
    );
    for mode in [AgentChatMode::Ask, AgentChatMode::Plan] {
        let (mut session, _) = CodexAppServerSession::start(CodexSessionConfig {
            working_directory: None,
            resume_thread_id: None,
            turn_options: options(mode),
            mcp_servers: None,
        })
        .unwrap();
        let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
        let _ = session
            .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
            .unwrap();
        assert_eq!(
            decode(&session.start_turn("hello").unwrap())["params"]["sandboxPolicy"],
            json!({"type":"readOnly","networkAccess":false})
        );
    }
}
#[path = "codex_session/default_model.rs"]
mod default_model;

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
