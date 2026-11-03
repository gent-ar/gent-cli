use super::*;

#[test]
fn v25_upgrade_is_additive_and_preserves_immutable_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.db");
    let (ledger, conversation) = populate(SqliteLedger::open(&path).unwrap());
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    drop(ledger);
    let db = rusqlite::Connection::open(&path).unwrap();
    let receipts: i64 = db
        .query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get(0))
        .unwrap();
    let events: i64 = db
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("DROP TABLE agent_chat_steer_targets; UPDATE gent_schema SET identity='gent-fresh-schema-v25';").unwrap();
    drop(db);
    let ledger = SqliteLedger::open(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        receipts
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        events
    );
    drop(db);
    request(&ledger, &conversation, &queued, &current, "after-upgrade").unwrap();
    assert!(claim(&ledger, &current).is_some());
}
