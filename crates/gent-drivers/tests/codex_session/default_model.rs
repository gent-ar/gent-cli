use super::*;

#[test]
fn default_model_omits_the_provider_field_so_codex_uses_its_configured_default() {
    let turn_options = CodexTurnOptions::from_selection(
        &AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "default".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Ask,
        },
        Some("/work"),
    )
    .unwrap();
    let (mut session, _) = CodexAppServerSession::start(CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: None,
        turn_options,
        mcp_servers: None,
    })
    .unwrap();
    let _ = session.receive(&json!({"id": 1, "result": {}})).unwrap();
    let _ = session
        .receive(&json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let turn = decode(&session.start_turn("hello").unwrap());
    assert!(turn["params"].get("model").is_none());
    assert_eq!(turn["params"]["effort"], "medium");
}
