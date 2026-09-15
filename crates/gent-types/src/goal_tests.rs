use serde_json::json;

use super::{
    GOAL_SCHEMA_VERSION, GoalBinding, GoalContractError, GoalProjection, GoalRecord, GoalStatus,
    GoalStatusReason, MAX_GOAL_OBJECTIVE_BYTES,
};
use crate::AgentChatConversationId;

fn active() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        reason: GoalStatusReason::UserSet,
        objective: "Ship the terminal workflow".into(),
        note: None,
        time_used_seconds: 30,
        active_since: Some(100),
        tokens_used: 12,
        token_budget: Some(1_000),
        turns_without_progress: 0,
        accounted_through_ordinal: 4,
        created_at: 70,
        updated_at: 100,
    }
}

#[test]
fn goal_record_has_a_closed_public_shape() {
    assert_eq!(
        serde_json::to_value(active()).unwrap(),
        json!({
            "schemaVersion": 2,
            "binding": { "goalId": "goal-1", "conversationId": "conversation-1" },
            "revision": 1,
            "status": "active",
            "reason": "userSet",
            "objective": "Ship the terminal workflow",
            "note": null,
            "timeUsedSeconds": 30,
            "activeSince": 100,
            "tokensUsed": 12,
            "tokenBudget": 1000,
            "turnsWithoutProgress": 0,
            "accountedThroughOrdinal": 4,
            "createdAt": 70,
            "updatedAt": 100
        })
    );
    let mut value = serde_json::to_value(active()).unwrap();
    value["providerSessionId"] = json!("never-public");
    assert!(serde_json::from_value::<GoalRecord>(value).is_err());
}

#[test]
fn active_time_excludes_every_span_outside_the_active_instant() {
    let goal = active();
    assert_eq!(goal.time_used_at(145), 75);
    let paused = GoalRecord {
        status: GoalStatus::Paused,
        active_since: None,
        ..active()
    };
    assert_eq!(paused.time_used_at(10_000), 30);
}

#[test]
fn active_instant_must_match_the_active_status() {
    let paused_with_instant = GoalRecord {
        status: GoalStatus::Paused,
        ..active()
    };
    assert_eq!(
        paused_with_instant.validate(),
        Err(GoalContractError::InvalidMetadata)
    );
    let active_without_instant = GoalRecord {
        active_since: None,
        ..active()
    };
    assert!(active_without_instant.validate().is_err());
    let oversized = GoalRecord {
        objective: "x".repeat(MAX_GOAL_OBJECTIVE_BYTES + 1),
        ..active()
    };
    assert!(oversized.validate().is_err());
    let zero_budget = GoalRecord {
        token_budget: Some(0),
        ..active()
    };
    assert!(zero_budget.validate().is_err());
}

#[test]
fn only_a_valid_active_ledger_goal_can_be_projected_to_an_adapter() {
    let projection = GoalProjection::from_active(&active()).unwrap();
    assert_eq!(projection.goal_id(), "goal-1");
    assert_eq!(projection.objective(), "Ship the terminal workflow");
    assert_eq!(
        serde_json::to_value(&projection).unwrap(),
        json!({
            "goalId": "goal-1",
            "objective": "Ship the terminal workflow",
            "tokensUsed": 12,
            "tokenBudget": 1000
        })
    );
    let complete = GoalRecord {
        status: GoalStatus::Complete,
        active_since: None,
        ..active()
    };
    assert_eq!(
        GoalProjection::from_active(&complete),
        Err(GoalContractError::InactiveGoal)
    );
    assert!(GoalStatus::Cleared.is_terminal());
    assert!(GoalStatus::BudgetLimited.is_resumable());
    assert!(!GoalStatus::Complete.is_resumable());
}

#[test]
fn budget_is_exhausted_only_at_or_past_its_bound() {
    let mut goal = active();
    assert!(!goal.budget_exhausted());
    goal.tokens_used = 1_000;
    assert!(goal.budget_exhausted());
    goal.token_budget = None;
    assert!(!goal.budget_exhausted());
}
