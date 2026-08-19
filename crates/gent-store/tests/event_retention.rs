use gent_ports::Ledger;
use gent_store::SqliteLedger;
use gent_types::{Event, HostEpoch, ReceiptId};
use serde_json::json;

fn event(id: u64) -> Event {
    Event {
        cursor: 0,
        event_id: format!("event-{id}"),
        receipt_id: ReceiptId(format!("receipt-{id}")),
        host_epoch: HostEpoch(1),
        kind: "fixture".into(),
        payload: json!({ "id": id }),
    }
}

#[test]
fn cursor_pages_survive_restart_without_retiring_events() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let first = ledger.append_event(&event(1)).unwrap();
    let second = ledger.append_event(&event(2)).unwrap();
    let third = ledger.append_event(&event(3)).unwrap();
    assert_eq!((first.cursor, second.cursor, third.cursor), (1, 2, 3));
    drop(ledger);

    let restarted = SqliteLedger::open(&path).unwrap();
    let page = restarted.read_event_page(0, 2).unwrap();
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(page.next_after_cursor, Some(second.cursor));
    let fourth = restarted.append_event(&event(4)).unwrap();
    assert_eq!(fourth.cursor, 4);
    let page = restarted.read_event_page(second.cursor, 2).unwrap();
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(page.next_after_cursor, None);
}

#[test]
fn pages_clamp_oversized_limits_and_keep_the_entire_history() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.append_event(&event(1)).unwrap();
    ledger.append_event(&event(2)).unwrap();

    for id in 3..=102 {
        ledger.append_event(&event(id)).unwrap();
    }
    let page = ledger.read_event_page(0, usize::MAX).unwrap();
    assert_eq!(page.events.len(), 100);
    assert_eq!(page.next_after_cursor, Some(100));
    let remaining = ledger.read_event_page(100, 100).unwrap();
    assert_eq!(
        remaining
            .events
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        [101, 102]
    );
    assert_eq!(remaining.next_after_cursor, None);
}
