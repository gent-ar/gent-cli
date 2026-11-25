use crate::SqliteLedger;

fn plan(query: &str) -> Vec<String> {
    let ledger = SqliteLedger::in_memory().unwrap();
    let connection = ledger.lock().unwrap();
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .unwrap();
    statement
        .query_map(["run"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn root_facts_are_read_through_their_partial_index() {
    assert_eq!(
        plan(super::ROOT_FACTS),
        ["SEARCH run_lifecycle_facts USING INDEX run_lifecycle_facts_root (run_id=?)"]
    );
}

#[test]
fn work_facts_are_read_through_their_partial_index() {
    assert_eq!(
        plan(super::WORK_FACTS),
        [
            "SEARCH f USING INDEX run_lifecycle_facts_work (run_id=?)",
            "SEARCH e USING INTEGER PRIMARY KEY (rowid=?)"
        ]
    );
}

#[test]
fn a_v31_ledger_gains_the_status_indexes_once() {
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    crate::sqlite::fresh_schema::apply(&mut connection).unwrap();
    connection
        .execute_batch(
            "DROP INDEX run_lifecycle_facts_root; DROP INDEX run_lifecycle_facts_work; \
             UPDATE gent_schema SET identity = 'gent-fresh-schema-v31' WHERE singleton = 1;",
        )
        .unwrap();
    crate::sqlite::fresh_schema::apply(&mut connection).unwrap();
    let indexes: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('run_lifecycle_facts_root', 'run_lifecycle_facts_work')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let identity: String = connection
        .query_row("SELECT identity FROM gent_schema", [], |row| row.get(0))
        .unwrap();
    assert_eq!((indexes, identity.as_str()), (2, "gent-fresh-schema-v35"));
}
