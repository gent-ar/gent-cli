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
    serde_json::from_slice(&frame[..frame.len() - 1]).unwrap()
}

#[test]
fn initialize_then_start_then_turn_uses_only_correlated_responses() {
    let (mut session, initialize) = CodexAppServerSession::start(config(None)).unwrap();
    assert_eq!(
        decode(&initialize),
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"gent","version":env!("CARGO_PKG_VERSION")},"capabilities":{}}})
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
        CodexSessionIngress::Ready
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
        json!({"id":2,"method":"thread/resume","params":{"cwd":"/work","threadId":"thread-1"}})
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
fn invalid_inputs_and_failed_responses_are_bounded_and_secret_free() {
    assert_eq!(
        CodexAppServerSession::start(CodexSessionConfig {
            working_directory: Some(String::new()),
            resume_thread_id: None,
            turn_options: options(AgentChatMode::Ask),
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
