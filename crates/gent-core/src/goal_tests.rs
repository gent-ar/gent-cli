use gent_types::{
    AgentChatConversationId, GoalRecord, GoalReportOutcome, GoalStatus, GoalStatusReason,
};

use crate::{
    GoalDraft, GoalRejection, GoalUserCommand, apply_user_command, continuation_prompt,
    continuation_request_id, create_goal, replaced_goal, reported_goal, stopped_goal,
};

pub(super) fn active(now: u64) -> GoalRecord {
    create_goal(
        GoalDraft {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            objective: "Ship the release".into(),
            token_budget: Some(1_000),
            accounted_through_ordinal: 2,
        },
        now,
    )
    .unwrap()
}

#[test]
fn a_new_goal_starts_active_at_its_creation_instant() {
    let goal = active(100);
    assert_eq!(goal.status, GoalStatus::Active);
    assert_eq!(goal.reason, GoalStatusReason::UserSet);
    assert_eq!(goal.active_since, Some(100));
    assert_eq!(
        (goal.created_at, goal.updated_at, goal.revision),
        (100, 100, 1)
    );
    assert_eq!(
        create_goal(
            GoalDraft {
                goal_id: "goal-2".into(),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                objective: " ".into(),
                token_budget: None,
                accounted_through_ordinal: 0,
            },
            100,
        ),
        Err(GoalRejection::InvalidValue)
    );
}

#[test]
fn pause_and_resume_accumulate_only_active_seconds() {
    let paused = apply_user_command(&active(100), 1, GoalUserCommand::Pause, 160).unwrap();
    assert_eq!(paused.status, GoalStatus::Paused);
    assert_eq!((paused.time_used_seconds, paused.active_since), (60, None));
    let resumed = apply_user_command(
        &paused,
        2,
        GoalUserCommand::Resume {
            accounted_through_ordinal: 9,
        },
        1_000,
    )
    .unwrap();
    assert_eq!(resumed.status, GoalStatus::Active);
    assert_eq!(resumed.reason, GoalStatusReason::UserResumed);
    assert_eq!(resumed.active_since, Some(1_000));
    assert_eq!(resumed.accounted_through_ordinal, 9);
    assert_eq!(resumed.time_used_at(1_030), 90);
}

#[test]
fn user_commands_are_revision_fenced_and_state_checked() {
    let goal = active(100);
    assert_eq!(
        apply_user_command(&goal, 7, GoalUserCommand::Pause, 110),
        Err(GoalRejection::RevisionMismatch)
    );
    assert_eq!(
        apply_user_command(
            &goal,
            1,
            GoalUserCommand::Resume {
                accounted_through_ordinal: 0
            },
            110
        ),
        Err(GoalRejection::NotResumable)
    );
    let cleared = apply_user_command(&goal, 1, GoalUserCommand::Clear, 110).unwrap();
    assert_eq!(cleared.status, GoalStatus::Cleared);
    assert_eq!(
        apply_user_command(&cleared, 2, GoalUserCommand::Clear, 120),
        Err(GoalRejection::NotActive)
    );
}

#[test]
fn clearing_a_finished_goal_dismisses_it_with_a_new_revision() {
    let complete = reported_goal(&active(100), GoalReportOutcome::Complete, None, 150).unwrap();
    let dismissed = apply_user_command(&complete, 2, GoalUserCommand::Clear, 170).unwrap();
    assert_eq!(dismissed.status, GoalStatus::Cleared);
    assert_eq!(dismissed.reason, GoalStatusReason::UserCleared);
    assert_eq!(dismissed.revision, 3);
    assert_eq!(dismissed.time_used_seconds, complete.time_used_seconds);
    assert_eq!(dismissed.active_since, None);
    assert_eq!(dismissed.updated_at, 170);
    assert_eq!(
        apply_user_command(&dismissed, 3, GoalUserCommand::Clear, 180),
        Err(GoalRejection::NotActive)
    );
    let blocked = reported_goal(&active(100), GoalReportOutcome::Blocked, None, 150).unwrap();
    assert_eq!(
        apply_user_command(&blocked, 2, GoalUserCommand::Clear, 160)
            .unwrap()
            .status,
        GoalStatus::Cleared
    );
}

#[test]
fn a_budget_limited_goal_cannot_resume_while_its_budget_is_spent() {
    let mut limited = apply_user_command(&active(100), 1, GoalUserCommand::Pause, 110).unwrap();
    limited.status = GoalStatus::BudgetLimited;
    limited.tokens_used = 1_000;
    assert_eq!(
        apply_user_command(
            &limited,
            2,
            GoalUserCommand::Resume {
                accounted_through_ordinal: 0
            },
            120
        ),
        Err(GoalRejection::BudgetExhausted)
    );
}

#[test]
fn stop_pauses_only_an_active_goal_with_an_explicit_reason() {
    let stopped = stopped_goal(&active(100), 130).unwrap();
    assert_eq!(stopped.status, GoalStatus::Paused);
    assert_eq!(stopped.reason, GoalStatusReason::UserStopped);
    assert_eq!(stopped_goal(&stopped, 140), None);
}

#[test]
fn model_reports_settle_only_an_active_goal() {
    let complete = reported_goal(
        &active(100),
        GoalReportOutcome::Complete,
        Some("All tests pass".into()),
        150,
    )
    .unwrap();
    assert_eq!(complete.status, GoalStatus::Complete);
    assert_eq!(complete.reason, GoalStatusReason::ModelCompleted);
    assert_eq!(complete.note.as_deref(), Some("All tests pass"));
    assert_eq!(
        reported_goal(&complete, GoalReportOutcome::Blocked, None, 160),
        Err(GoalRejection::NotActive)
    );
    let blocked = reported_goal(&active(100), GoalReportOutcome::Blocked, None, 150).unwrap();
    assert_eq!(blocked.status, GoalStatus::Blocked);
    assert_eq!(blocked.reason, GoalStatusReason::ModelBlocked);
}

#[test]
fn replacing_a_goal_clears_only_an_unsettled_predecessor() {
    let replaced = replaced_goal(&active(100), 120).unwrap();
    assert_eq!(replaced.status, GoalStatus::Cleared);
    assert_eq!(replaced.reason, GoalStatusReason::UserReplaced);
    assert_eq!(replaced_goal(&replaced, 130), None);
}

#[test]
fn continuation_identity_is_derived_from_the_accounted_turn() {
    let goal = active(100);
    let request_id = continuation_request_id(&goal);
    assert_eq!(request_id, "goal-continuation:goal-1:2");
    let prompt = continuation_prompt(&goal);
    assert!(prompt.contains("\"Ship the release\""));
    assert!(prompt.contains("gent_goal_update"));
    assert!(prompt.contains("goalId \"goal-1\""));
}
