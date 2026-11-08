use std::sync::{Arc, mpsc};

use super::SingleFlight;

fn waiters(flights: &SingleFlight<u32>, key: &str) -> usize {
    flights
        .lock()
        .get(key)
        .map_or(0, |flight| Arc::strong_count(flight) - 1)
}

#[test]
fn a_request_arriving_during_a_flight_shares_its_result_instead_of_running_again() {
    let flights = Arc::new(SingleFlight::new());
    let (started, on_start) = mpsc::channel();
    let (release, on_release) = mpsc::channel::<()>();
    let leader = {
        let flights = Arc::clone(&flights);
        std::thread::spawn(move || {
            flights.run("repo", move || {
                started.send(()).unwrap();
                on_release.recv().unwrap();
                7
            })
        })
    };
    on_start.recv().unwrap();
    let follower = {
        let flights = Arc::clone(&flights);
        std::thread::spawn(move || flights.run("repo", || panic!("the flight ran twice")))
    };
    while waiters(&flights, "repo") < 2 {
        std::thread::yield_now();
    }
    release.send(()).unwrap();

    assert_eq!(leader.join().unwrap(), 7);
    assert_eq!(follower.join().unwrap(), 7);
    assert_eq!(waiters(&flights, "repo"), 0);
}

#[test]
fn a_request_after_a_flight_lands_runs_fresh_work_and_other_keys_never_wait() {
    let flights = SingleFlight::new();

    assert_eq!(flights.run("repo", || 1), 1);
    assert_eq!(flights.run("repo", || 2), 2);
    assert_eq!(flights.run("other", || 3), 3);
}
