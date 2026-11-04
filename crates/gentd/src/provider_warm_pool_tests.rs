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
