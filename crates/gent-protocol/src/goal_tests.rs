use super::{GOAL_CAPABILITY, GoalFrame, GoalFrameError, MAX_GOAL_FRAME_BYTES};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, GoalTransition, HostEpoch,
};
use serde_json::json;

fn binding() -> GoalBinding {
    GoalBinding {
        goal_id: "goal-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
    }
}

fn goal() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: binding(),
        revision: 1,
        status: GoalStatus::Active,
        summary: "Make the terminal prompt workflow usable".into(),
    }
}

#[test]
fn goal_exchange_is_typed_and_capability_gated() {
    let frame = GoalFrame::Transition {
        request_id: "request-1".into(),
        transition: GoalTransition {
            binding: binding(),
            expected_revision: 1,
            host_epoch: HostEpoch(1),
            next_status: GoalStatus::Completed,
        },
    };
    assert_eq!(frame.validate(), Ok(()));
    assert_eq!(GOAL_CAPABILITY, "goal-v1");
}

#[test]
fn frames_reject_unknown_and_provider_fields() {
    let frame = json!({
        "type": "create", "body": {
            "requestId": "request-1", "goal": {
                "schemaVersion": 1, "binding": {
                    "goalId": "goal-1", "conversationId": "conversation-1", "runId": "run-1"
                }, "revision": 1, "status": "active", "summary": "Ship it",
                "providerSessionId": "must-not-cross-the-contract"
            }
        }
    });
    assert!(serde_json::from_value::<GoalFrame>(frame).is_err());
}

#[test]
fn correlated_read_response_rechecks_its_binding() {
    let mismatch = GoalFrame::Goal {
        request_id: "request-1".into(),
        binding: GoalBinding {
            goal_id: "goal-2".into(),
            ..binding()
        },
        goal: Some(goal()),
    };
    assert_eq!(mismatch.validate(), Err(GoalFrameError::BindingMismatch));
}

#[test]
fn list_response_is_bounded_to_its_conversation() {
    let mismatch = GoalFrame::Goals {
        request_id: "request-1".into(),
        conversation_id: AgentChatConversationId("conversation-2".into()),
        goals: vec![goal()],
    };
    assert_eq!(mismatch.validate(), Err(GoalFrameError::BindingMismatch));

    let oversized = GoalFrame::Created {
        request_id: "request-1".into(),
        goal: GoalRecord {
            summary: "x".repeat(MAX_GOAL_FRAME_BYTES),
            ..goal()
        },
    };
    assert_eq!(oversized.validate(), Err(GoalFrameError::InvalidValue));
}
