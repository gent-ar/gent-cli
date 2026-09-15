use gent_core::MAX_CONTINUATIONS_WITHOUT_PROGRESS;
use gent_ports::PendingPermissionLedger;
use gent_runtime::GoalResult;
use gent_types::{
    AgentChatPromptDisposition::Send, AgentChatProvider, DurableTurnPhase, GoalReportOutcome,
    GoalStatus, GoalStatusReason,
};

use super::fake_cli::FakeClaudeDaemon;
use crate::goal_pursuit_host::test_support::GoalDriver;

fn tick(daemon: &mut FakeClaudeDaemon, driver: &GoalDriver, now: u64) -> usize {
    let epoch = daemon.epoch;
    driver.tick(&mut daemon.router, epoch, now).admitted.len()
}

#[test]
fn claude_keeps_pursuing_through_its_live_stream_until_the_progress_ceiling() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, _) = daemon.conversation("pursuit");
    let seed = daemon.prompt(&conversation, "Remember CODE-GOAL", Send);
    daemon.drive_until("seed turn", |daemon| daemon.phase(&seed).is_terminal());
    let driver = GoalDriver::new(&daemon.ledger, &conversation);
    let goal = driver.set("Report the remembered code", daemon.epoch);
    let mut admitted = 0;
    for now in 0..10 {
        admitted += tick(&mut daemon, &driver, 1_001 + now);
        daemon.drive_until("continuation settles", |_| driver.idle());
        if driver.current().status != GoalStatus::Active {
            break;
        }
    }
    let blocked = driver.current();
    assert_eq!(
        (blocked.status, blocked.reason),
        (GoalStatus::Blocked, GoalStatusReason::NoProgressLimit)
    );
    assert_eq!(admitted, usize::from(MAX_CONTINUATIONS_WITHOUT_PROGRESS));
    let continuations = driver
        .turns()
        .into_iter()
        .filter(|turn| turn.continuation_of.as_deref() == Some(goal.binding.goal_id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(continuations.len(), admitted);
    assert!(
        continuations
            .iter()
            .all(|turn| turn.phase == DurableTurnPhase::Completed)
    );
    assert_eq!(daemon.launches().len(), 1);
    let projection = daemon.projection(&conversation);
    assert!(projection.iter().any(|event| {
        event.payload["kind"] == "assistantMessage"
            && event.payload["text"]
                .as_str()
                .is_some_and(|text| text.contains("recall: CODE-GOAL"))
    }));
    assert!(projection.iter().any(|event| {
        event.payload["activity"]["type"] == "goalUpdated"
            && event.payload["activity"]["goal"]["reason"] == "noProgressLimit"
    }));
    let origins = projection
        .iter()
        .filter(|event| event.payload["kind"] == "userMessage")
        .map(|event| event.payload["origin"]["kind"].clone())
        .collect::<Vec<_>>();
    assert_eq!(origins.len(), 1 + admitted);
    assert_eq!(origins[0], "user");
    assert!(origins[1..].iter().all(|kind| kind == "goalContinuation"));
    assert!(
        projection
            .iter()
            .any(|event| { event.payload["origin"]["goalId"] == goal.binding.goal_id.as_str() })
    );
    assert_eq!(tick(&mut daemon, &driver, 2_000), 0);
}

#[test]
fn claude_turn_usage_including_cached_tokens_trips_the_goal_token_budget() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, _) = daemon.conversation("budget");
    let driver = GoalDriver::new(&daemon.ledger, &conversation);
    driver.set_with_budget("Keep working", Some(30_000), daemon.epoch);
    let mut admitted = 0;
    for now in 0..5 {
        admitted += tick(&mut daemon, &driver, 1_001 + now);
        daemon.drive_until("continuation settles", |_| driver.idle());
        if driver.current().status != GoalStatus::Active {
            break;
        }
    }
    let limited = driver.current();
    assert_eq!(
        (
            limited.status,
            limited.reason,
            limited.tokens_used,
            admitted
        ),
        (
            GoalStatus::BudgetLimited,
            GoalStatusReason::TokenBudgetExhausted,
            44_820,
            2
        )
    );
}

#[test]
fn a_model_report_during_a_live_claude_turn_completes_the_goal_without_another_turn() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.conversation("report");
    let driver = GoalDriver::new(&daemon.ledger, &conversation);
    let goal = driver.set("Ask PERMISSION before finishing", daemon.epoch);
    assert_eq!(tick(&mut daemon, &driver, 1_001), 1);
    daemon.drive_until("continuation waits for permission", |daemon| {
        daemon
            .ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_some()
    });
    let GoalResult::Goal(Some(complete)) = driver
        .goals
        .report(
            &goal.binding.goal_id,
            GoalReportOutcome::Complete,
            Some("Done".into()),
            daemon.epoch,
            1_010,
        )
        .unwrap()
    else {
        panic!("a report during the live turn settles the goal");
    };
    assert_eq!(complete.status, GoalStatus::Complete);
    daemon
        .router
        .interrupt_run(AgentChatProvider::Claude, &run.0)
        .unwrap();
    daemon.drive_until("reported turn settles", |_| driver.idle());
    assert_eq!(tick(&mut daemon, &driver, 1_020), 0);
    assert_eq!(driver.current().status, GoalStatus::Complete);
}

#[test]
fn a_user_stop_pauses_pursuit_and_a_provider_failure_blocks_it() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.conversation("stop");
    let driver = GoalDriver::new(&daemon.ledger, &conversation);
    driver.set("Wait for PERMISSION", daemon.epoch);
    assert_eq!(tick(&mut daemon, &driver, 1_001), 1);
    daemon.drive_until("continuation waits for permission", |daemon| {
        daemon
            .ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_some()
    });
    let stopped = driver
        .goals
        .stop(&conversation, daemon.epoch, 1_030)
        .unwrap()
        .unwrap();
    assert_eq!(stopped.reason, GoalStatusReason::UserStopped);
    daemon
        .router
        .interrupt_run(AgentChatProvider::Claude, &run.0)
        .unwrap();
    daemon.drive_until("stopped turn settles", |_| driver.idle());
    assert_eq!(tick(&mut daemon, &driver, 1_040), 0);
    assert_eq!(driver.current().status, GoalStatus::Paused);

    let (failing, _) = daemon.conversation("failing");
    let seed = daemon.prompt(&failing, "Bind the session", Send);
    daemon.drive_until("seed turn", |daemon| daemon.phase(&seed).is_terminal());
    let failing_driver = GoalDriver::new(&daemon.ledger, &failing);
    failing_driver.set("ROTATE the session", daemon.epoch);
    let epoch = daemon.epoch;
    assert_eq!(
        failing_driver
            .tick(&mut daemon.router, epoch, 1_001)
            .admitted
            .len(),
        1
    );
    daemon.drive_until("failed continuation settles", |_| failing_driver.idle());
    assert!(
        failing_driver
            .tick(&mut daemon.router, epoch, 1_002)
            .admitted
            .is_empty()
    );
    let blocked = failing_driver.current();
    assert_eq!(
        (blocked.status, blocked.reason),
        (GoalStatus::Blocked, GoalStatusReason::ProviderFailed)
    );
}
