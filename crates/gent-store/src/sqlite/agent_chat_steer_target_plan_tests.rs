use crate::SqliteLedger;

#[test]
fn live_dispatched_turn_reads_only_indexed_rows() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let connection = ledger.lock().unwrap();
    let mut statement = connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN SELECT m.run_id, m.turn_id {} ORDER BY t.sequence DESC LIMIT 1",
            super::LIVE_DISPATCHED_TURN
        ))
        .unwrap();
    let plan = statement
        .query_map(rusqlite::params!["c", "r", 1], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .all(|step| !step.starts_with("SCAN") || step.contains("TEMP B-TREE")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|step| step
                == "SEARCH d USING INDEX agent_chat_prompt_dispatches_pending (state=?)"),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|step| step.contains("conversation_activity_facts")),
        "{plan:?}"
    );
}
