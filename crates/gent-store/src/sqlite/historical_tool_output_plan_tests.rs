use crate::SqliteLedger;

#[test]
fn activity_page_reads_only_the_requested_run_lifecycle_facts() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let connection = ledger.lock().unwrap();
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {}", super::ACTIVITY_PAGE))
        .unwrap();
    let plan = statement
        .query_map(rusqlite::params!["c", "r", 0, 10], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter().any(|step| step.starts_with(
            "SEARCH f USING INDEX run_lifecycle_facts_by_run_cursor (run_id=? AND cursor>?)"
        )),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|step| step.starts_with("SEARCH b USING INDEX sqlite_autoindex_normalized_session_batches_1 (lifecycle_event_id=?)")),
        "{plan:?}"
    );
}
