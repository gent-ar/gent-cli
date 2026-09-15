mod goal_support;

use gent_core::MAX_CONTINUATIONS_WITHOUT_PROGRESS;
use gent_ports::{AgentChatPromptDispatchLedger, GoalLedger};
use gent_runtime::GoalControl;
use gent_types::{
    AgentChatPromptDisposition, AgentChatProvider, DurableTurnPhase, GoalStatus, GoalStatusReason,
};
use goal_support::{Harness, Refusing, Unreleased};

#[test]
fn a_settled_turn_admits_exactly_one_continuation_carrying_the_goal() {
    let mut harness = Harness::new();
    let goal = harness.set("Ship the release", None, 100);
    let tick = harness.tick(101);
    assert_eq!(tick.admitted.len(), 1);
    let continuation = &tick.admitted[0];
    assert!(harness.tick(102).admitted.is_empty());
    let started = harness.start_next();
    assert_eq!(started.message.message_id, continuation.message_id);
    assert!(started.message.text.contains("\"Ship the release\""));
    assert!(started.message.text.contains(&goal.binding.goal_id));
    assert!(harness.tick(103).admitted.is_empty());
    harness.work(&started, 500, true);
    harness.settle(&started, DurableTurnPhase::Completed);
    let next = harness.tick(104);
    assert_eq!(next.admitted.len(), 1);
    assert_ne!(next.admitted[0].message_id, started.message.message_id);
    let accounted = harness.current();
    assert_eq!(accounted.tokens_used, 500);
    assert_eq!(accounted.status, GoalStatus::Active);
}

#[test]
fn a_waiting_user_prompt_runs_before_the_next_continuation() {
    let mut harness = Harness::new();
    harness.set("Ship", None, 100);
    harness.tick(101);
    let continuation = harness.start_next();
    let user = harness.user_prompt(AgentChatPromptDisposition::Queue);
    harness.settle(&continuation, DurableTurnPhase::Completed);
    assert!(harness.tick(102).admitted.is_empty());
    let started = harness.start_next();
    assert_eq!(started.message.message_id, user.message.message_id);
    assert!(harness.tick(103).admitted.is_empty());
    harness.settle(&started, DurableTurnPhase::Completed);
    assert_eq!(harness.tick(104).admitted.len(), 1);
}

#[test]
fn pause_leaves_the_running_turn_and_resume_schedules_one_continuation() {
    let harness = Harness::new();
    let goal = harness.set("Ship", None, 100);
    harness.tick(101);
    let running = harness.start_next();
    let paused = harness.goals.control(
        &harness.conversation,
        &goal.binding.goal_id,
        goal.revision,
        GoalControl::Pause,
        harness.epoch,
        110,
    );
    assert!(
        matches!(paused, Ok(gent_runtime::GoalResult::Goal(Some(ref paused))) if paused.status == GoalStatus::Paused)
    );
    harness.settle(&running, DurableTurnPhase::Completed);
    assert!(harness.tick(120).admitted.is_empty());
    let current = harness.current();
    assert_eq!(current.time_used_seconds, 10);
    harness
        .goals
        .control(
            &harness.conversation,
            &goal.binding.goal_id,
            current.revision,
            GoalControl::Resume,
            harness.epoch,
            500,
        )
        .unwrap();
    assert_eq!(harness.tick(501).admitted.len(), 1);
    assert!(harness.tick(502).admitted.is_empty());
    assert_eq!(harness.current().time_used_at(530), 40);
}

#[test]
fn a_steer_inside_a_pursuit_turn_keeps_the_goal_but_a_user_stop_pauses_it() {
    let mut harness = Harness::new();
    harness.set("Ship", None, 100);
    harness.tick(101);
    let running = harness.start_next();
    harness.user_prompt(AgentChatPromptDisposition::Queue);
    harness.settle(&running, DurableTurnPhase::Interrupted);
    assert!(harness.tick(102).admitted.is_empty());
    assert_eq!(harness.current().status, GoalStatus::Active);
    let steered = harness.start_next();
    harness.settle(&steered, DurableTurnPhase::Completed);
    assert_eq!(harness.tick(103).admitted.len(), 1);
    let continuation = harness.start_next();
    let stopped = harness
        .goals
        .stop(&harness.conversation, harness.epoch, 110)
        .unwrap()
        .expect("a user stop pauses the goal");
    assert_eq!(
        (stopped.status, stopped.reason),
        (GoalStatus::Paused, GoalStatusReason::UserStopped)
    );
    harness.settle(&continuation, DurableTurnPhase::Interrupted);
    assert!(harness.tick(111).admitted.is_empty());
    assert_eq!(
        harness
            .goals
            .stop(&harness.conversation, harness.epoch, 112)
            .unwrap(),
        None
    );
}

#[test]
fn the_model_completion_report_is_accepted_only_during_a_pursuit_turn() {
    let mut harness = Harness::new();
    let goal = harness.set("Ship", None, 100);
    let report = |harness: &Harness, now| {
        harness
            .goals
            .report(
                &goal.binding.goal_id,
                gent_types::GoalReportOutcome::Complete,
                Some("Released".into()),
                harness.epoch,
                now,
            )
            .unwrap()
    };
    assert_eq!(
        report(&harness, 101),
        gent_runtime::GoalResult::Rejected(gent_core::GoalRejection::NoActiveTurn)
    );
    harness.tick(102);
    let running = harness.start_next();
    let gent_runtime::GoalResult::Goal(Some(complete)) = report(&harness, 103) else {
        panic!("in-flight report must settle the goal");
    };
    assert_eq!(
        (complete.status, complete.reason),
        (GoalStatus::Complete, GoalStatusReason::ModelCompleted)
    );
    harness.work(&running, 10, true);
    harness.settle(&running, DurableTurnPhase::Completed);
    assert!(harness.tick(104).admitted.is_empty());
    assert_eq!(harness.current().status, GoalStatus::Complete);
    let clear = |harness: &Harness, revision| {
        harness
            .goals
            .control(
                &harness.conversation,
                &goal.binding.goal_id,
                revision,
                GoalControl::Clear,
                harness.epoch,
                105,
            )
            .unwrap()
    };
    let gent_runtime::GoalResult::Goal(Some(dismissed)) = clear(&harness, complete.revision) else {
        panic!("a completed goal must be clearable");
    };
    assert_eq!(
        (dismissed.status, dismissed.reason, dismissed.revision),
        (
            GoalStatus::Cleared,
            GoalStatusReason::UserCleared,
            complete.revision + 1
        )
    );
    assert_eq!(
        harness.goals.current(&harness.conversation).unwrap(),
        gent_runtime::GoalResult::Goal(None)
    );
    assert_eq!(
        clear(&harness, dismissed.revision),
        gent_runtime::GoalResult::Rejected(gent_core::GoalRejection::NotActive)
    );
}

#[test]
fn provider_failure_and_refused_admission_block_without_spinning() {
    let harness = Harness::new();
    harness.set("Ship", None, 100);
    harness.tick(101);
    let failed = harness.start_next();
    harness.settle(&failed, DurableTurnPhase::Failed);
    let tick = harness.tick(102);
    assert!(tick.admitted.is_empty());
    assert_eq!(harness.current().reason, GoalStatusReason::ProviderFailed);
    assert!(harness.tick(103).admitted.is_empty());

    let refused = Harness::new();
    refused.set("Ship", None, 100);
    let tick = refused
        .pursuit
        .tick(refused.epoch, 101, &mut Refusing)
        .unwrap();
    assert_eq!(tick.settled[0].reason, GoalStatusReason::AdmissionHeld);
    assert!(
        refused
            .pursuit
            .tick(refused.epoch, 102, &mut Refusing)
            .unwrap()
            .settled
            .is_empty()
    );
}

#[test]
fn the_token_budget_and_the_progress_ceiling_stop_pursuit() {
    let mut budget = Harness::new();
    budget.set("Ship", Some(1_000), 100);
    budget.tick(101);
    budget.complete(1_200, true);
    let tick = budget.tick(102);
    assert!(tick.admitted.is_empty());
    assert_eq!(budget.current().status, GoalStatus::BudgetLimited);

    let mut idle = Harness::new();
    idle.set("Ship", None, 100);
    for now in 0..u64::from(MAX_CONTINUATIONS_WITHOUT_PROGRESS) {
        assert_eq!(idle.tick(101 + now).admitted.len(), 1);
        idle.complete(10, false);
    }
    assert!(idle.tick(200).admitted.is_empty());
    let blocked = idle.current();
    assert_eq!(
        (blocked.status, blocked.reason),
        (GoalStatus::Blocked, GoalStatusReason::NoProgressLimit)
    );
}

#[test]
fn restart_resumes_pursuit_exactly_once_from_the_ledger() {
    let mut harness = Harness::new();
    let goal = harness.set("Ship", None, 100);
    harness.tick(101);
    harness.start_next();
    harness.restart();
    let first = harness.tick(200);
    let second = harness.tick(201);
    assert_eq!(first.admitted.len(), 1);
    assert!(second.admitted.is_empty());
    let continuations = harness
        .ledger
        .goal_turns(&harness.conversation.0, 0)
        .unwrap()
        .into_iter()
        .filter(|turn| !turn.phase.is_terminal())
        .collect::<Vec<_>>();
    assert_eq!(continuations.len(), 1);
    assert_eq!(
        continuations[0].continuation_of.as_deref(),
        Some(goal.binding.goal_id.as_str())
    );

    let mut unwoken = Harness::new();
    unwoken.set("Ship", None, 100);
    unwoken
        .pursuit
        .tick(unwoken.epoch, 101, &mut Unreleased)
        .unwrap();
    unwoken.restart();
    assert_eq!(unwoken.tick(200).admitted.len(), 1);
    assert!(unwoken.tick(201).admitted.is_empty());
    assert_eq!(
        unwoken
            .ledger
            .goal_turns(&unwoken.conversation.0, 0)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_goal_survives_provider_switches_and_continues_on_the_selected_run() {
    let mut harness = Harness::new();
    let goal = harness.set("Ship across providers", None, 100);
    harness.tick(101);
    let claude = harness.start_next();
    harness.work(&claude, 100, true);
    harness.settle(&claude, DurableTurnPhase::Completed);
    let codex_run = harness.switch(AgentChatProvider::Codex, "gpt-5.6");
    let tick = harness.tick(102);
    assert_eq!(tick.admitted.len(), 1);
    assert_eq!(tick.admitted[0].run_id, codex_run);
    assert!(
        harness
            .ledger
            .claim_agent_chat_prompt_dispatch(
                "provider-host",
                harness.epoch,
                AgentChatProvider::Claude
            )
            .unwrap()
            .is_none()
    );
    let codex = harness.start_next_for(AgentChatProvider::Codex);
    assert!(codex.message.text.contains(&goal.binding.goal_id));
    harness.work(&codex, 200, true);
    harness.settle(&codex, DurableTurnPhase::Completed);
    let claurst_run = harness.switch(AgentChatProvider::Claurst, "qwen3-4b");
    let tick = harness.tick(103);
    assert_eq!(tick.admitted[0].run_id, claurst_run);
    let claurst = harness.start_next_for(AgentChatProvider::Claurst);
    assert_eq!(claurst.run_id, claurst_run);
    let current = harness.current();
    assert_eq!(current.binding, goal.binding);
    assert_eq!(
        (current.status, current.tokens_used),
        (GoalStatus::Active, 300)
    );
}
