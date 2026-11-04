use super::{WarmRun, evictions};

fn run<'a>(run_id: &'a str, conversation_id: &'a str, idle: bool, last: u64) -> WarmRun<'a> {
    WarmRun {
        run_id,
        conversation_id,
        idle,
        last_dispatch: last,
    }
}

#[test]
fn a_new_conversation_keeps_every_idle_process_while_the_pool_has_room() {
    let runs = [run("run-a", "a", true, 1), run("run-b", "b", true, 2)];

    assert!(evictions(runs, "run-c", "c", 4).is_empty());
}

#[test]
fn a_full_pool_stops_the_least_recently_dispatched_idle_process() {
    let runs = [
        run("run-a", "a", true, 3),
        run("run-b", "b", true, 1),
        run("run-c", "c", false, 0),
    ];

    assert_eq!(evictions(runs, "run-d", "d", 3), ["run-b"]);
}

#[test]
fn a_busy_process_is_never_stopped_to_make_room() {
    let runs = [run("run-a", "a", false, 1), run("run-b", "b", false, 2)];

    assert!(evictions(runs, "run-c", "c", 2).is_empty());
}

#[test]
fn a_superseded_run_of_the_same_conversation_is_stopped_even_with_room() {
    let runs = [run("run-a", "a", true, 1), run("run-b", "b", true, 2)];

    assert_eq!(evictions(runs, "run-a-child", "a", 4), ["run-a"]);
}

#[test]
fn a_capacity_of_zero_still_keeps_one_process() {
    let runs = [run("run-a", "a", true, 1)];

    assert_eq!(evictions(runs, "run-b", "b", 0), ["run-a"]);
}

fn idle(run_id: &str, minutes: u64, served: bool, current: bool) -> super::IdleRun<'_> {
    super::IdleRun {
        run_id,
        idle_for: std::time::Duration::from_secs(minutes * 60),
        served,
        current,
    }
}

#[test]
fn a_warm_up_for_a_run_already_owned_is_already_warm_or_blocked_by_its_turn() {
    use gent_protocol::ProviderWarmOutcome;
    let idle_run = [run("run-a", "a", true, 1)];
    let busy_run = [run("run-a", "a", false, 1)];
    let busy_sibling = [run("run-a", "a", false, 1)];

    assert_eq!(
        super::admission(idle_run, "run-a", "a", 4),
        Err(ProviderWarmOutcome::AlreadyWarm)
    );
    assert_eq!(
        super::admission(busy_run, "run-a", "a", 4),
        Err(ProviderWarmOutcome::TurnActive)
    );
    assert_eq!(
        super::admission(busy_sibling, "run-a-child", "a", 4),
        Err(ProviderWarmOutcome::TurnActive)
    );
}

#[test]
fn a_warm_up_into_a_pool_of_busy_runs_is_refused_but_may_evict_an_idle_one() {
    use gent_protocol::ProviderWarmOutcome;
    let busy = [run("run-a", "a", false, 1), run("run-b", "b", false, 2)];
    let mixed = [run("run-a", "a", false, 1), run("run-b", "b", true, 2)];

    assert_eq!(
        super::admission(busy, "run-c", "c", 2),
        Err(ProviderWarmOutcome::PoolFull)
    );
    assert_eq!(
        super::admission(mixed, "run-c", "c", 2),
        Ok(vec!["run-b".to_owned()])
    );
}

#[test]
fn an_unprompted_warm_process_expires_before_a_served_one() {
    let runs = [
        idle("fresh", 1, false, true),
        idle("unprompted", 2, false, true),
        idle("served", 9, true, true),
        idle("served-long", 10, true, true),
    ];

    assert_eq!(super::expired(runs), ["unprompted", "served-long"]);
}

#[test]
fn a_superseded_run_is_reaped_immediately() {
    assert_eq!(
        super::expired([idle("old", 0, false, false), idle("new", 0, false, true)]),
        ["old"]
    );
}
