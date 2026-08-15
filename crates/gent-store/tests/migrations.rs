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
    assert!(
        reopened
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = 3",
                [],
                |_| Ok(()),
            )
            .is_ok()
    );
    assert!(
        reopened
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = 4",
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

#[test]
fn v13_attachment_uploads_gain_a_transfer_owned_staging_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("attachments-v13.db");
    drop(SqliteLedger::open(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE turn_attachments;
             DROP TABLE attachments;
             DROP TABLE receipts;
             DELETE FROM schema_migrations WHERE version IN (14, 15, 16);
             CREATE TABLE receipts (
                 idempotency_key TEXT PRIMARY KEY NOT NULL, receipt_id TEXT NOT NULL UNIQUE,
                 status TEXT NOT NULL, host_epoch INTEGER NOT NULL
             );
             CREATE TABLE attachments (
                 attachment_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL UNIQUE,
                 receipt_id TEXT NOT NULL UNIQUE, host_epoch INTEGER NOT NULL, state TEXT NOT NULL,
                 received_bytes INTEGER NOT NULL, display_name TEXT NOT NULL, media_type TEXT NOT NULL,
                 byte_len INTEGER NOT NULL, digest_sha256 TEXT NOT NULL UNIQUE, storage_key TEXT NOT NULL UNIQUE
             );
             CREATE TABLE turn_attachments (
                 turn_id TEXT NOT NULL REFERENCES turns(turn_id), attachment_id TEXT NOT NULL REFERENCES attachments(attachment_id),
                 PRIMARY KEY (turn_id, attachment_id)
             );
             INSERT INTO attachments VALUES ('attachment-1', 'key-1', 'receipt-1', 1, 'uploading', 0,
                 'notes.txt', 'text/plain', 4, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');",
        )
        .unwrap();
    drop(connection);
    drop(SqliteLedger::open(&path).unwrap());
    let reopened = Connection::open(path).unwrap();
    assert_eq!(
        reopened
            .query_row(
                "SELECT staging_key FROM attachments WHERE attachment_id = 'attachment-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}
