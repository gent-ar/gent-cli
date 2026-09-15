use super::{GOAL_CAPABILITY, GoalFrame, GoalFrameError, GoalRejectionCode, MAX_GOAL_FRAME_BYTES};
use gent_types::{
    AgentChatConversationId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord, GoalReportOutcome,
    GoalStatus, GoalStatusReason,
};
use serde_json::json;

fn conversation() -> AgentChatConversationId {
    AgentChatConversationId("conversation-1".into())
}

fn goal() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: conversation(),
        },
        revision: 3,
        status: GoalStatus::Paused,
        reason: GoalStatusReason::UserStopped,
        objective: "Make the terminal prompt workflow usable".into(),
        note: None,
        time_used_seconds: 42,
        active_since: None,
        tokens_used: 900,
        token_budget: Some(5_000),
        turns_without_progress: 1,
        accounted_through_ordinal: 7,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_042,
    }
}

#[test]
fn every_goal_intent_has_a_closed_request_shape() {
    assert_eq!(GOAL_CAPABILITY, "goal-v1");
    let set = GoalFrame::Set {
        request_id: "request-1".into(),
        conversation_id: conversation(),
        objective: "Ship it".into(),
        token_budget: Some(10_000),
    };
    assert_eq!(
        serde_json::to_value(&set).unwrap(),
        json!({"type": "set", "body": {"requestId": "request-1", "conversationId": "conversation-1", "objective": "Ship it", "tokenBudget": 10000}})
    );
    assert_eq!(
        serde_json::from_value::<GoalFrame>(json!({"type": "set", "body": {"requestId": "request-1", "conversationId": "conversation-1", "objective": "Ship it"}})).unwrap(),
        GoalFrame::Set {
            request_id: "request-1".into(),
            conversation_id: conversation(),
            objective: "Ship it".into(),
            token_budget: None,
        }
    );
    for kind in ["pause", "resume", "clear"] {
        let frame = serde_json::from_value::<GoalFrame>(json!({"type": kind, "body": {"requestId": "request-2", "conversationId": "conversation-1", "goalId": "goal-1", "expectedRevision": 3}})).unwrap();
        assert_eq!(frame.validate(), Ok(()));
        assert!(frame.is_client_request());
    }
    let report = GoalFrame::Report {
        request_id: "request-3".into(),
        goal_id: "goal-1".into(),
        outcome: GoalReportOutcome::Blocked,
        note: Some("Needs credentials".into()),
    };
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({"type": "report", "body": {"requestId": "request-3", "goalId": "goal-1", "outcome": "blocked", "note": "Needs credentials"}})
    );
    assert!(
        serde_json::from_value::<GoalFrame>(json!({"type": "read", "body": {"requestId": "request-4", "conversationId": "conversation-1", "runId": "run-1"}})).is_err()
    );
}

#[test]
fn replies_carry_the_current_goal_or_a_typed_rejection() {
    let reply = GoalFrame::Goal {
        request_id: "request-1".into(),
        conversation_id: conversation(),
        goal: Some(goal()),
    };
    assert_eq!(reply.validate(), Ok(()));
    assert!(!reply.is_client_request());
    let rejected = GoalFrame::Rejected {
        request_id: "request-1".into(),
        code: GoalRejectionCode::GoalRevisionMismatch,
    };
    assert_eq!(
        serde_json::to_value(&rejected).unwrap(),
        json!({"type": "rejected", "body": {"requestId": "request-1", "code": "goalRevisionMismatch"}})
    );
    let foreign = GoalFrame::Goal {
        request_id: "request-1".into(),
        conversation_id: AgentChatConversationId("conversation-2".into()),
        goal: Some(goal()),
    };
    assert_eq!(foreign.validate(), Err(GoalFrameError::BindingMismatch));
}

#[test]
fn goal_frames_reject_unsafe_values_and_oversized_payloads() {
    let zero_budget = GoalFrame::Set {
        request_id: "request-1".into(),
        conversation_id: conversation(),
        objective: "Ship".into(),
        token_budget: Some(0),
    };
    assert_eq!(zero_budget.validate(), Err(GoalFrameError::InvalidValue));
    let control = GoalFrame::Pause {
        request_id: "\n".into(),
        conversation_id: conversation(),
        goal_id: "goal-1".into(),
        expected_revision: 1,
    };
    assert_eq!(control.validate(), Err(GoalFrameError::InvalidIdentifier));
    let oversized = GoalFrame::Report {
        request_id: "request-1".into(),
        goal_id: "goal-1".into(),
        outcome: GoalReportOutcome::Complete,
        note: Some("x".repeat(MAX_GOAL_FRAME_BYTES)),
    };
    assert_eq!(oversized.validate(), Err(GoalFrameError::InvalidValue));
}
