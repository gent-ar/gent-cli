use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, FrozenConversationContext, GOAL_SCHEMA_VERSION, GoalBinding,
    GoalProjection, GoalRecord, GoalStatus, RunVersionLock,
};

use super::{ClaudeRunStart, ClaudeRunnerError, input_frame};
use crate::claude_turn_options::ClaudeTurnOptions;

#[test]
fn fresh_claude_context_cannot_resume_a_native_session() {
    let start = ClaudeRunStart {
        fresh_context: Some(FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation".into(),
        ))),
        resume_session_id: Some("native-session".into()),
        ..start()
    };
    assert!(matches!(
        input_frame(&start),
        Err(ClaudeRunnerError::InvalidPrompt)
    ));
}

#[test]
fn cleared_fresh_claude_context_uses_no_native_session_field() {
    let start = ClaudeRunStart {
        fresh_context: Some(FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation".into(),
        ))),
        ..start()
    };
    let value: serde_json::Value = serde_json::from_slice(&input_frame(&start).unwrap()).unwrap();
    assert!(value.get("session_id").is_none());
    assert!(
        value["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("prompt")
    );
}

#[test]
fn fresh_context_keeps_the_gent_owned_active_goal() {
    let start = ClaudeRunStart {
        fresh_context: Some(FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation".into(),
        ))),
        goal: Some(
            GoalProjection::from_active(&GoalRecord {
                schema_version: GOAL_SCHEMA_VERSION,
                binding: GoalBinding {
                    goal_id: "goal".into(),
                    conversation_id: AgentChatConversationId("conversation".into()),
                    run_id: AgentChatRunId("run".into()),
                },
                revision: 1,
                status: GoalStatus::Active,
                summary: "finish the task".into(),
            })
            .unwrap(),
        ),
        ..start()
    };
    let value: serde_json::Value = serde_json::from_slice(&input_frame(&start).unwrap()).unwrap();
    let text = value["message"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"goalId\":\"goal\""));
    assert!(text.ends_with("User prompt:\nprompt"));
    assert!(value.get("session_id").is_none());
}

fn start() -> ClaudeRunStart {
    ClaudeRunStart {
        run_id: "run".into(),
        lock: RunVersionLock {
            provider: "claude".into(),
            canonical_path: "/locked/claude".into(),
            file_identity: "1:2".into(),
            digest_sha256: "a".repeat(64),
            version: "1".into(),
            compatibility_entry: "entry".into(),
        },
        prompt: "prompt".into(),
        turn_options: ClaudeTurnOptions::from_selection(&AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-sonnet".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Ask,
        })
        .unwrap(),
        goal: None,
        fresh_context: None,
        resume_session_id: None,
    }
}
