use super::*;

#[test]
fn default_model_resolves_the_provider_published_default_before_the_thread_starts() {
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
        .receive(&json!({"id": 2, "result": {"data": [{"model": "published-default", "isDefault": true}], "nextCursor": null}}))
        .unwrap();
    let _ = session
        .receive(&json!({"id": 3, "result": {"thread": {"id": "thread-1"}}}))
        .unwrap();
    let turn = decode(&session.start_turn("hello").unwrap());
    assert_eq!(turn["params"]["model"], "published-default");
    assert_eq!(turn["params"]["effort"], "medium");
}
