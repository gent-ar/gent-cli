use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    FrozenConversationContext, GOAL_SCHEMA_VERSION, GoalBinding, GoalProjection, GoalRecord,
    GoalStatus, RunVersionLock,
};

use super::{
    ClaudeRunStart, ClaudeRunnerError,
    input::{follow_up_input_frame, input_frame, message_uuid, steer_input_frame},
};
use crate::claude_turn_options::ClaudeTurnOptions;

#[test]
fn fresh_claude_context_cannot_resume_a_native_session() {
    let start = ClaudeRunStart {
        fresh_context: Some(FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation".into(),
        ))),
        intent: crate::launch_spec::LaunchIntent::Resume {
            session_id: "native-session".into(),
        },
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
                },
                revision: 1,
                status: GoalStatus::Active,
                reason: gent_types::GoalStatusReason::UserSet,
                objective: "finish the task".into(),
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

#[test]
fn claude_input_keeps_image_blocks_after_the_text_prompt() {
    let start = ClaudeRunStart {
        content: vec![
            serde_json::json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}}),
        ],
        ..start()
    };
    let value: serde_json::Value = serde_json::from_slice(&input_frame(&start).unwrap()).unwrap();
    assert_eq!(value["message"]["content"][0]["type"], "text");
    assert_eq!(value["message"]["content"][1]["type"], "image");
    assert_eq!(
        value["message"]["content"][1]["source"],
        serde_json::json!({"type":"base64","media_type":"image/png","data":"YWJj"})
    );
}

#[test]
fn follow_up_input_is_a_new_user_frame_without_a_resume_session() {
    let frame = follow_up_input_frame(
        "message-2",
        "next prompt",
        None,
        &[serde_json::json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}})],
    )
    .unwrap();
    assert!(frame.ends_with(b"\n"));
    let value: serde_json::Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(value["type"], "user");
    assert!(value.get("session_id").is_none());
    assert_eq!(value["message"]["content"][0]["type"], "text");
    assert_eq!(value["message"]["content"][1]["type"], "image");
}

fn start() -> ClaudeRunStart {
    ClaudeRunStart {
        run_id: "run".into(),
        message_id: "message".into(),
        lock: RunVersionLock {
            provider: "claude".into(),
            canonical_path: "/locked/claude".into(),
            file_identity: "1:2".into(),
            digest_sha256: "a".repeat(64),
            version: "1".into(),
            compatibility_entry: "entry".into(),
        },
        prompt: "prompt".into(),
        content: Vec::new(),
        turn_options: ClaudeTurnOptions::from_selection(&AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-sonnet".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Ask,
        })
        .unwrap(),
        goal: None,
        fresh_context: None,
        intent: crate::launch_spec::LaunchIntent::Start,
        workspace_root: "/workspace".into(),
        workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
        mcp_config: None,
    }
}

#[test]
fn every_user_frame_carries_the_stable_uuid_of_its_message() {
    let uuid = message_uuid("message-1");
    assert_eq!(uuid, message_uuid("message-1"));
    assert_ne!(uuid, message_uuid("message-2"));
    let groups = uuid.split('-').map(str::len).collect::<Vec<_>>();
    assert_eq!(groups, [8, 4, 4, 4, 12]);
    assert!(uuid.chars().nth(14) == Some('4') && uuid.chars().nth(19) == Some('8'));
    let uuid_of = |frame: Vec<u8>| {
        serde_json::from_slice::<serde_json::Value>(&frame).unwrap()["uuid"].clone()
    };
    let first = ClaudeRunStart {
        message_id: "message-1".into(),
        ..start()
    };
    assert_eq!(uuid_of(input_frame(&first).unwrap()), uuid);
    assert_eq!(
        uuid_of(follow_up_input_frame("message-1", "next", None, &[]).unwrap()),
        uuid
    );
    assert_eq!(
        uuid_of(steer_input_frame("message-1", "also this", &[]).unwrap()),
        uuid
    );
    let missing = ClaudeRunStart {
        message_id: String::new(),
        ..start()
    };
    assert!(matches!(
        input_frame(&missing),
        Err(ClaudeRunnerError::InvalidPrompt)
    ));
}

#[test]
fn a_warm_process_first_input_matches_the_cold_launch_frame() {
    let fresh = start();
    let mut resumed = start();
    resumed.intent = crate::launch_spec::LaunchIntent::Resume {
        session_id: "session-1".into(),
    };

    assert_eq!(
        follow_up_input_frame(&fresh.message_id, &fresh.prompt, None, &[]).unwrap(),
        input_frame(&fresh).unwrap()
    );
    assert_eq!(
        super::input::resumed_first_input_frame(
            &resumed.message_id,
            &resumed.prompt,
            None,
            &[],
            "session-1"
        )
        .unwrap(),
        input_frame(&resumed).unwrap()
    );
}
