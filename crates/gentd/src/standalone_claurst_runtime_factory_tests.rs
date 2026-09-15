use std::path::PathBuf;

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};

use super::RuntimeIdentity;

fn identity(selection: &AgentChatSelection, tool_source_ids: &[String]) -> RuntimeIdentity {
    RuntimeIdentity::new(
        selection,
        PathBuf::from("/workspace"),
        PermissionMode::AskEveryTime,
        Some("mcp".into()),
        tool_source_ids,
    )
}

#[test]
fn a_mode_effort_or_tool_source_change_needs_a_relaunched_local_runtime() {
    let agent = AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    };
    let running = identity(&agent, &["b".into(), "a".into()]);
    assert_eq!(running, identity(&agent, &["a".into(), "b".into()]));

    let mut ask = agent.clone();
    ask.mode = AgentChatMode::Ask;
    let mut high = agent.clone();
    high.effort = AgentChatEffort::High;
    for changed in [
        identity(&ask, &["a".into(), "b".into()]),
        identity(&high, &["a".into(), "b".into()]),
        identity(&agent, &["a".into()]),
    ] {
        assert_ne!(running, changed);
    }
}
