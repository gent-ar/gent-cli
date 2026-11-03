use super::*;

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
