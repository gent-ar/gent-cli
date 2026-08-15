use gent_ports::Ledger;
use gent_store::SqliteLedger;
use gent_types::{EventResume, HostEpoch};
use rusqlite::{Connection, params};

#[test]
fn legacy_ledger_is_upgraded_without_losing_epoch_or_events() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE host_state (singleton INTEGER PRIMARY KEY, epoch INTEGER NOT NULL);
             INSERT INTO host_state (singleton, epoch) VALUES (1, 7);
             CREATE TABLE events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL);
             INSERT INTO events (event_id, receipt_id, host_epoch, kind, payload) VALUES ('legacy-event', 'legacy-receipt', 7, 'legacy', '{}');",
        )
        .unwrap();
    drop(connection);

    let ledger = SqliteLedger::open(&path).unwrap();
    assert_eq!(ledger.host_ingress().unwrap().epoch, HostEpoch(7));
    assert!(matches!(
        ledger.resume_events(0).unwrap(),
        EventResume::Delta { events } if events.len() == 1 && events[0].kind == "legacy"
    ));
    drop(ledger);

    let reopened = Connection::open(path).unwrap();
    assert_eq!(
        reopened
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        reopened
            .query_row(
                "SELECT ingress FROM host_state WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "open"
    );
    assert!(
        reopened
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = 2",
                [],
                |_| Ok(()),
            )
            .is_ok()
    );
}

#[test]
fn checksum_tampering_is_rejected_before_the_ledger_opens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tampered.db");
    drop(SqliteLedger::open(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
            params!["not-a-valid-checksum"],
        )
        .unwrap();
    drop(connection);

    assert!(SqliteLedger::open(&path).is_err());
}
