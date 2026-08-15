use gent_ports::Ledger;
use gent_store::SqliteLedger;
use gent_types::{Event, EventResume, EventSnapshot, HostEpoch, ReceiptId};
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

fn snapshot(cursor: u64) -> EventSnapshot {
    EventSnapshot {
        cursor,
        host_epoch: HostEpoch(1),
        schema_version: 1,
        payload: json!({ "projected": cursor }),
    }
}

#[test]
fn compaction_survives_restart_and_resyncs_stale_cursors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let first = ledger.append_event(&event(1)).unwrap();
    let second = ledger.append_event(&event(2)).unwrap();
    let third = ledger.append_event(&event(3)).unwrap();
    assert_eq!((first.cursor, second.cursor, third.cursor), (1, 2, 3));
    ledger.compact_events(&snapshot(second.cursor)).unwrap();
    drop(ledger);

    let restarted = SqliteLedger::open(&path).unwrap();
    assert!(matches!(
        restarted.resume_events(0).unwrap(),
        EventResume::Resync { snapshot, events }
            if snapshot.cursor == 2 && events.iter().map(|event| event.cursor).eq([3])
    ));
    let fourth = restarted.append_event(&event(4)).unwrap();
    assert_eq!(fourth.cursor, 4);
    assert!(matches!(
        restarted.resume_events(2).unwrap(),
        EventResume::Delta { events }
            if events.iter().map(|event| event.cursor).eq([3, 4])
    ));
}

#[test]
fn invalid_compaction_keeps_the_existing_event_feed_intact() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.append_event(&event(1)).unwrap();
    ledger.append_event(&event(2)).unwrap();

    assert!(ledger.compact_events(&snapshot(3)).is_err());
    assert!(matches!(
        ledger.resume_events(0).unwrap(),
        EventResume::Delta { events } if events.len() == 2
    ));
}
