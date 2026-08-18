use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, GoalTransition, HostEpoch,
};

use super::{
    ActiveGoalSelection, GoalControlContext, GoalControlEffect, GoalControlEvent,
    GoalControlRejection, GoalControlState, reduce_goal_control, select_active_goal,
};

fn context() -> GoalControlContext {
    GoalControlContext {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        host_epoch: HostEpoch(2),
    }
}

fn binding() -> GoalBinding {
    GoalBinding {
        goal_id: "goal-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
    }
}

fn record() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: binding(),
        revision: 1,
        status: GoalStatus::Active,
        summary: "Finish the terminal implementation".into(),
    }
}

fn transition(binding: GoalBinding, revision: u64, status: GoalStatus) -> GoalTransition {
    GoalTransition {
        binding,
        expected_revision: revision,
        host_epoch: HostEpoch(2),
        next_status: status,
    }
}

#[test]
fn empty_state_exposes_no_goal_and_selects_no_active_goal() {
    let state = GoalControlState::default();
    assert_eq!(state.goal(), None);
    assert_eq!(
        select_active_goal(&[], "conversation-1", "run-1"),
        ActiveGoalSelection::None
    );
}

#[test]
fn creation_rejects_malformed_stale_and_non_initial_goals() {
    let invalid = GoalRecord {
        summary: String::new(),
        ..record()
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Create(invalid),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::InvalidRecord)
    );

    let stale_conversation = GoalRecord {
        binding: GoalBinding {
            conversation_id: AgentChatConversationId("conversation-2".into()),
            ..binding()
        },
        ..record()
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Create(stale_conversation),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::StaleConversation)
    );

    let stale_run = GoalRecord {
        binding: GoalBinding {
            run_id: AgentChatRunId("run-2".into()),
            ..binding()
        },
        ..record()
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Create(stale_run),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::StaleRun)
    );

    for invalid_initial in [
        GoalRecord {
            revision: 2,
            ..record()
        },
        GoalRecord {
            status: GoalStatus::Completed,
            ..record()
        },
    ] {
        let (_, effect) = reduce_goal_control(
            GoalControlState::default(),
            &context(),
            GoalControlEvent::Create(invalid_initial),
        );
        assert_eq!(
            effect,
            GoalControlEffect::Rejected(GoalControlRejection::ActiveStatusRequired)
        );
    }
}

#[test]
fn identical_create_is_idempotent_and_conflicting_create_is_rejected() {
    let existing = record();
    let state = GoalControlState::new(Some(existing.clone()));
    let (unchanged_state, effect) = reduce_goal_control(
        state.clone(),
        &context(),
        GoalControlEvent::Create(existing.clone()),
    );
    assert_eq!(unchanged_state, state);
    assert_eq!(effect, GoalControlEffect::Unchanged(existing));

    let conflicting = GoalRecord {
        summary: "A different user goal".into(),
        ..record()
    };
    let (_, effect) = reduce_goal_control(state, &context(), GoalControlEvent::Create(conflicting));
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::GoalAlreadyExists)
    );
}

#[test]
fn transitions_reject_invalid_stale_and_missing_goals() {
    let invalid = GoalTransition {
        expected_revision: 0,
        ..transition(binding(), 1, GoalStatus::Completed)
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Transition(invalid),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::InvalidTransition)
    );

    let stale_conversation = GoalBinding {
        conversation_id: AgentChatConversationId("conversation-2".into()),
        ..binding()
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Transition(transition(stale_conversation, 1, GoalStatus::Completed)),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::StaleConversation)
    );

    let stale_run = GoalBinding {
        run_id: AgentChatRunId("run-2".into()),
        ..binding()
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Transition(transition(stale_run, 1, GoalStatus::Completed)),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::StaleRun)
    );

    let (_, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Transition(transition(binding(), 1, GoalStatus::Completed)),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::GoalMissing)
    );
}

#[test]
fn transitions_fence_binding_revision_and_active_status_before_persisting() {
    let state = GoalControlState::new(Some(record()));
    let other_binding = GoalBinding {
        goal_id: "goal-2".into(),
        ..binding()
    };
    let (_, effect) = reduce_goal_control(
        state.clone(),
        &context(),
        GoalControlEvent::Transition(transition(other_binding, 1, GoalStatus::Completed)),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::BindingMismatch)
    );

    let (_, effect) = reduce_goal_control(
        state.clone(),
        &context(),
        GoalControlEvent::Transition(transition(binding(), 2, GoalStatus::Completed)),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::RevisionMismatch)
    );

    let (_, effect) = reduce_goal_control(
        state,
        &context(),
        GoalControlEvent::Transition(transition(binding(), 1, GoalStatus::Active)),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::ActiveStatusRequired)
    );
}
