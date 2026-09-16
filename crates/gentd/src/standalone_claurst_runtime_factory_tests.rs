use std::path::PathBuf;

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};

use super::launch::conversation_servers;
use crate::standalone_claurst_runtime_identity::{RuntimeIdentity, RuntimeReuse};
use crate::standalone_mcp_config::StandaloneMcpConfig;

fn identity(selection: &AgentChatSelection, tool_source_ids: &[String]) -> RuntimeIdentity {
    identity_for(selection, "conversation-1", tool_source_ids)
}

fn identity_for(
    selection: &AgentChatSelection,
    conversation_id: &str,
    tool_source_ids: &[String],
) -> RuntimeIdentity {
    RuntimeIdentity::new(
        selection,
        conversation_id.into(),
        PathBuf::from("/workspace"),
        PermissionMode::AskEveryTime,
        Some("mcp".into()),
        tool_source_ids,
    )
}

#[test]
fn a_mode_or_effort_change_relaunches_while_a_tool_source_change_only_rebinds() {
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
    ] {
        assert_eq!(changed.reuse_from(&running), RuntimeReuse::Relaunch);
    }
    assert_eq!(
        identity(&agent, &["a".into()]).reuse_from(&running),
        RuntimeReuse::RebindSession
    );
}

fn claurst() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn config(directory: &std::path::Path) -> StandaloneMcpConfig {
    let path = directory.join("standalone-mcp.json");
    std::fs::write(
        &path,
        br#"{"mcpServers":{"gent-chat":{"command":"gent","args":["mcp","chat"]},"gent-goal":{"command":"gent","args":["mcp","goal"]}}}"#,
    )
    .unwrap();
    StandaloneMcpConfig::load(&path).unwrap()
}

fn launched(config: &StandaloneMcpConfig, conversation_id: &str) -> Vec<Vec<serde_json::Value>> {
    [
        conversation_servers(
            Some(config),
            &[],
            conversation_id,
            StandaloneMcpConfig::claurst_servers,
            StandaloneMcpConfig::selected_claurst_servers,
        )
        .unwrap(),
        conversation_servers(
            Some(config),
            &[],
            conversation_id,
            StandaloneMcpConfig::claurst_settings_servers,
            StandaloneMcpConfig::selected_claurst_settings_servers,
        )
        .unwrap(),
    ]
    .into_iter()
    .collect()
}

fn arguments(entries: &[serde_json::Value], name: &str) -> serde_json::Value {
    entries
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("{name} is launched"))["args"]
        .clone()
}

#[test]
fn the_same_conversation_reuses_its_running_local_runtime_untouched() {
    let agent = claurst();
    assert_eq!(
        identity_for(&agent, "conversation-1", &["a".into(), "b".into()]).reuse_from(
            &identity_for(&agent, "conversation-1", &["b".into(), "a".into()])
        ),
        RuntimeReuse::Ready
    );
}

#[test]
fn another_conversation_replaces_only_the_claurst_agent_and_keeps_the_model_loaded() {
    let agent = claurst();
    assert_eq!(
        identity_for(&agent, "conversation-2", &[]).reuse_from(&identity_for(
            &agent,
            "conversation-1",
            &[]
        )),
        RuntimeReuse::RebindSession
    );
}

#[test]
fn each_conversation_launches_with_its_own_chat_server_and_never_the_previous_one() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());

    for entries in launched(&config, "conversation-1") {
        assert_eq!(
            arguments(&entries, "gent-chat"),
            serde_json::json!(["mcp", "chat", "--conversation-id", "conversation-1"])
        );
        assert_eq!(
            arguments(&entries, "gent-goal"),
            serde_json::json!(["mcp", "goal"])
        );
    }
    for entries in launched(&config, "conversation-2") {
        let chat = arguments(&entries, "gent-chat");
        assert_eq!(
            chat,
            serde_json::json!(["mcp", "chat", "--conversation-id", "conversation-2"])
        );
        assert!(!chat.to_string().contains("conversation-1"));
    }
}
