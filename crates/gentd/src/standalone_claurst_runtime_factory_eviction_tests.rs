use std::time::{Duration, Instant};

use super::idle_victim;

fn sessions(rows: &[(&str, bool, u64)]) -> Vec<(String, bool, Instant)> {
    let start = Instant::now();
    rows.iter()
        .map(|(conversation, idle, age)| {
            (
                (*conversation).to_owned(),
                *idle,
                start + Duration::from_secs(*age),
            )
        })
        .collect()
}

fn victim(rows: &[(&str, bool, u64)], starting: &str, busy: &[&str]) -> Option<String> {
    let rows = sessions(rows);
    let busy = busy
        .iter()
        .map(|busy| (*busy).to_owned())
        .collect::<Vec<_>>();
    idle_victim(
        rows.iter()
            .map(|(conversation, idle, used)| (conversation, *idle, *used)),
        starting,
        &busy,
    )
}

#[test]
fn a_conversation_waiting_on_a_permission_is_never_evicted_even_when_oldest() {
    let rows = [
        ("waiting", false, 0),
        ("idle", true, 5),
        ("recent", true, 9),
    ];
    assert_eq!(victim(&rows, "new", &["waiting"]), Some("idle".into()));
}

#[test]
fn a_busy_conversation_is_kept_even_when_its_agent_reports_idle() {
    let rows = [("compacting", true, 0), ("idle", true, 5)];
    assert_eq!(victim(&rows, "new", &["compacting"]), Some("idle".into()));
}

#[test]
fn nothing_is_evicted_when_every_other_session_is_busy_or_starting() {
    let rows = [
        ("waiting", false, 0),
        ("running", true, 1),
        ("new", true, 2),
    ];
    assert_eq!(victim(&rows, "new", &["waiting", "running"]), None);
}
