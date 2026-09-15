use gent_runtime::GoalResult;
use gent_types::{
    AgentChatProvider, DurableTurnPhase, GoalDispatchState, GoalReportOutcome, GoalStatus,
};

use super::fake_cli::FakeCodexDaemon;
use crate::goal_pursuit_host::test_support::GoalDriver;

fn tick(daemon: &mut FakeCodexDaemon, driver: &GoalDriver, now: u64) -> usize {
    let epoch = daemon.epoch;
    driver.tick(&mut daemon.router, epoch, now).admitted.len()
}

fn in_flight(driver: &GoalDriver) -> bool {
    driver
        .turns()
        .iter()
        .any(|turn| turn.dispatch == GoalDispatchState::InFlight)
}

#[test]
fn codex_runs_continuations_as_gentd_admitted_turns_and_accepts_a_live_report() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, run) = daemon.conversation("pursuit");
    let driver = GoalDriver::new(&daemon.ledger, &conversation);
    let goal = driver.set("WAIT until the release is verified", daemon.epoch);
    assert_eq!(tick(&mut daemon, &driver, 1_001), 1);
    daemon.drive_until("continuation streams output", |daemon| {
        in_flight(&driver)
            && daemon.projection(&conversation).iter().any(|event| {
                event.payload["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("working"))
            })
    });
    let started = daemon.requests("turn/start");
    assert_eq!(started.len(), 1);
    assert!(
        started[0].to_string().contains(&goal.binding.goal_id),
        "the Codex turn carries the goal continuation"
    );
    assert_eq!(tick(&mut daemon, &driver, 1_002), 0);
    let GoalResult::Goal(Some(complete)) = driver
        .goals
        .report(
            &goal.binding.goal_id,
            GoalReportOutcome::Complete,
            None,
            daemon.epoch,
            1_003,
        )
        .unwrap()
    else {
        panic!("a live report settles the goal");
    };
    assert_eq!(complete.status, GoalStatus::Complete);
    daemon
        .router
        .interrupt_run(AgentChatProvider::Codex, &run.0)
        .unwrap();
    daemon.drive_until("reported turn settles", |_| driver.idle());
    assert_eq!(tick(&mut daemon, &driver, 1_004), 0);
    assert_eq!(daemon.requests("turn/start").len(), 1);
}

#[test]
fn daemon_restart_resumes_codex_pursuit_exactly_once() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, _) = daemon.conversation("restart");
    let driver = GoalDriver::new(&daemon.ledger, &conversation);
    driver.set("WAIT for the migration", daemon.epoch);
    assert_eq!(tick(&mut daemon, &driver, 1_001), 1);
    daemon.drive_until("continuation turn starts", |_| in_flight(&driver));
    daemon.restart();
    let resumed = GoalDriver::new(&daemon.ledger, &conversation);
    assert_eq!(resumed.turns()[0].phase, DurableTurnPhase::Failed);
    assert_eq!(resumed.turns()[0].dispatch, GoalDispatchState::Unprovable);
    assert_eq!(tick(&mut daemon, &resumed, 2_001), 1);
    assert_eq!(tick(&mut daemon, &resumed, 2_002), 0);
    daemon.drive_until("resumed continuation starts", |_| in_flight(&resumed));
    assert_eq!(tick(&mut daemon, &resumed, 2_003), 0);
    let turns = resumed.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns
            .iter()
            .filter(|turn| !turn.phase.is_terminal())
            .count(),
        1
    );
    assert_eq!(resumed.current().status, GoalStatus::Active);
}
