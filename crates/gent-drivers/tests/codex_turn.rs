use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    GOAL_SCHEMA_VERSION, GoalBinding, GoalProjection, GoalRecord, GoalStatus,
    NormalizedLifecycleSignal, NormalizedProviderEvent, WorkPhase,
};
use serde_json::{Value, json};

fn config() -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: None,
        turn_options: CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            Some("/work"),
        )
        .unwrap(),
        mcp_servers: None,
    }
}

fn frames(effects: &[CodexTurnEffect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Write(frame) => {
                let mut frame: Value = serde_json::from_slice(&frame[..frame.len() - 1]).ok()?;
                assert_eq!(frame["jsonrpc"], json!("2.0"));
                frame.as_object_mut()?.remove("jsonrpc");
                Some(frame)
            }
            CodexTurnEffect::Fact(_)
            | CodexTurnEffect::ControlRequest(_)
            | CodexTurnEffect::Steer(_) => None,
        })
        .collect()
}

fn goal(revision: u64) -> GoalProjection {
    GoalProjection::from_active(&GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        reason: gent_types::GoalStatusReason::UserSet,
        objective: format!("Finish the durable task {revision}"),
        note: None,
        time_used_seconds: 0,
        active_since: Some(1),
        tokens_used: 0,
        token_budget: None,
        turns_without_progress: 0,
        accounted_through_ordinal: 0,
        created_at: 1,
        updated_at: 1,
    })
    .unwrap()
}

#[path = "codex_turn/children.rs"]
mod children;
#[path = "codex_turn/control.rs"]
mod control;
#[path = "codex_turn/lifecycle.rs"]
mod lifecycle;
#[path = "codex_turn/prewarm.rs"]
mod prewarm;
