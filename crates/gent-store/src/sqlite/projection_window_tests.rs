use rusqlite::{Connection, StatementStatus, params};

use super::{
    CARRIED_TURNS, CONVERSATION_RUNS, LATEST_ACTIVITY, LATEST_SUMMARY, TURN_CURSOR_PAGE, TURN_FACTS,
};
use crate::SqliteLedger;
use crate::sqlite::agent_chat_projection_ledger::COLUMNS;
use crate::sqlite::run_live_status::WORK_FACTS;

const FACTS_PER_TURN: u64 = 30;

fn open_child(connection: &Connection, turn: u64) {
    let lifecycle = gent_types::NormalizedSessionLifecycle::Event {
        event: gent_types::NormalizedProviderEvent::ChildStarted {
            child_id: format!("child-{turn}"),
            parent_tool_use_id: "tool".into(),
        },
    };
    let payload = serde_json::json!({"conversationId": "conversation", "runId": "run", "turnId": format!("turn-{turn}"), "lifecycle": lifecycle});
    connection
        .execute(
            "INSERT INTO events (event_id, receipt_id, host_epoch, kind, payload) VALUES (?1, 'receipt', 1, 'normalizedSessionLifecycle', ?2)",
            params![format!("child-{turn}"), payload.to_string()],
        )
        .unwrap();
    let fact = gent_types::RunLifecycleFact {
        run_id: "run".into(),
        event_id: format!("child-{turn}"),
        host_epoch: gent_types::HostEpoch(1),
        cursor: u64::try_from(connection.last_insert_rowid()).unwrap(),
        lifecycle,
    };
    connection
        .execute(
            "INSERT INTO run_lifecycle_facts (run_id, cursor, event_id, host_epoch, payload) VALUES ('run', ?1, ?2, 1, ?3)",
            params![fact.cursor, fact.event_id, serde_json::to_string(&fact).unwrap()],
        )
        .unwrap();
}

fn seed(ledger: &SqliteLedger, turns: u64, child_turn: Option<u64>) {
    let connection = ledger.lock().unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF; BEGIN; INSERT INTO runs (run_id, provider, conversation_id) VALUES ('run', 'claude', 'conversation');")
        .unwrap();
    for turn in 1..=turns {
        let phase = if turn == turns { "active" } else { "completed" };
        connection
            .execute(
                "INSERT INTO turns (turn_id, conversation_id, run_id, sequence, phase) VALUES (?1, 'conversation', 'run', ?2, ?3)",
                params![format!("turn-{turn}"), turn, phase],
            )
            .unwrap();
        for fact in 0..FACTS_PER_TURN {
            let ordinal = (turn - 1) * FACTS_PER_TURN + fact + 1;
            let (kind, payload) = if fact % 2 == 0 {
                (
                    "transcript",
                    serde_json::json!({"cursor": ordinal, "eventId": format!("event-{ordinal}"), "turnId": format!("turn-{turn}"), "runId": "run", "kind": "assistantMessage", "text": "text", "isPartial": 0}),
                )
            } else {
                (
                    "activity",
                    serde_json::json!({"conversationId": "conversation", "runId": "run", "turnId": format!("turn-{turn}"), "activity": {"type": "turnStarted", "conversationId": "conversation", "runId": "run", "turnId": format!("turn-{turn}"), "hostEpoch": 1, "cursor": ordinal}}),
                )
            };
            connection
                .execute(
                    "INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES ('conversation', ?1, ?2, ?3)",
                    params![format!("source-{ordinal}"), kind, payload.to_string()],
                )
                .unwrap();
        }
        if child_turn == Some(turn) {
            open_child(&connection, turn);
        }
    }
    connection
        .execute_batch("COMMIT; PRAGMA foreign_keys = ON;")
        .unwrap();
}

fn plan(connection: &Connection, query: &str, values: impl rusqlite::Params) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .unwrap();
    statement
        .query_map(values, |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn steps(connection: &Connection, query: &str, values: impl rusqlite::Params) -> (i32, i32) {
    let mut statement = connection.prepare(query).unwrap();
    let mut rows = statement.query(values).unwrap();
    while rows.next().unwrap().is_some() {}
    drop(rows);
    (
        statement.get_status(StatementStatus::VmStep),
        statement.get_status(StatementStatus::FullscanStep),
    )
}

fn window_reads(connection: &Connection, from: u64) -> Vec<(String, (i32, i32))> {
    let turn = format!("turn-{}", from / FACTS_PER_TURN);
    let facts = format!("{COLUMNS} {TURN_FACTS}");
    let latest = format!("{COLUMNS} {LATEST_ACTIVITY}");
    let summary = format!("{COLUMNS} {LATEST_SUMMARY}");
    let mut reads = CARRIED_TURNS
        .iter()
        .map(|query| {
            (
                (*query).to_owned(),
                steps(connection, query, params!["conversation", from]),
            )
        })
        .collect::<Vec<_>>();
    reads.push((
        facts.clone(),
        steps(connection, &facts, params!["conversation", turn, from]),
    ));
    reads.push((
        latest.clone(),
        steps(
            connection,
            &latest,
            params!["conversation", "goalUpdated", from, 1],
        ),
    ));
    reads.push((
        summary.clone(),
        steps(connection, &summary, params!["conversation", from]),
    ));
    reads.push((
        TURN_CURSOR_PAGE.to_owned(),
        steps(
            connection,
            TURN_CURSOR_PAGE,
            params!["conversation", i64::MAX, 20],
        ),
    ));
    reads.push((
        CONVERSATION_RUNS.to_owned(),
        steps(connection, CONVERSATION_RUNS, params!["conversation", from]),
    ));
    reads.push((
        WORK_FACTS.to_owned(),
        steps(connection, WORK_FACTS, params!["run"]),
    ));
    reads
}

#[test]
fn every_window_read_searches_an_index() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let connection = ledger.lock().unwrap();
    let mut plans = CARRIED_TURNS
        .iter()
        .flat_map(|query| plan(&connection, query, params!["conversation", 9]))
        .collect::<Vec<_>>();
    plans.extend(plan(
        &connection,
        &format!("{COLUMNS} {TURN_FACTS}"),
        params!["conversation", "turn", 9],
    ));
    plans.extend(plan(
        &connection,
        &format!("{COLUMNS} {LATEST_ACTIVITY}"),
        params!["conversation", "goalUpdated", 9, 1],
    ));
    plans.extend(plan(
        &connection,
        &format!("{COLUMNS} {LATEST_SUMMARY}"),
        params!["conversation", 9],
    ));
    plans.extend(plan(
        &connection,
        TURN_CURSOR_PAGE,
        params!["conversation", 9, 20],
    ));
    plans.extend(plan(
        &connection,
        CONVERSATION_RUNS,
        params!["conversation", 9],
    ));
    plans.extend(plan(&connection, WORK_FACTS, params!["run"]));
    assert!(
        plans.iter().all(|step| step.starts_with("SEARCH ")),
        "{plans:#?}"
    );
    assert!(
        plans.iter().all(|step| !step.contains("TEMP B-TREE")),
        "{plans:#?}"
    );
}

#[test]
fn the_window_read_cost_is_flat_in_history_length() {
    let small = SqliteLedger::in_memory().unwrap();
    seed(&small, 100, Some(1));
    let large = SqliteLedger::in_memory().unwrap();
    seed(&large, 1000, Some(1));
    let small_reads = window_reads(&small.lock().unwrap(), 98 * FACTS_PER_TURN + 1);
    let large_reads = window_reads(&large.lock().unwrap(), 998 * FACTS_PER_TURN + 1);
    for ((query, small), (_, large)) in small_reads.iter().zip(&large_reads) {
        assert_eq!(large.1, 0, "{query} scanned a table");
        assert!(
            large.0 <= small.0 + 64,
            "{query}: {} steps over 30,000 facts against {} over 3,000",
            large.0,
            small.0
        );
    }
}

#[test]
fn turn_boundaries_page_newest_first_and_the_carry_holds_only_live_turns() {
    let ledger = SqliteLedger::in_memory().unwrap();
    seed(&ledger, 5, None);
    let first = ledger
        .conversation_turn_cursors("conversation", None, 2)
        .unwrap();
    assert_eq!(
        first
            .turns
            .iter()
            .map(|turn| (
                turn.turn_id.as_str(),
                turn.first_projection_cursor,
                turn.first_transcript_cursor,
                turn.first_activity_cursor
            ))
            .collect::<Vec<_>>(),
        [
            ("turn-5", 121, Some(121), Some(122)),
            ("turn-4", 91, Some(91), Some(92))
        ]
    );
    let next = ledger
        .conversation_turn_cursors("conversation", first.next_before_cursor, 200)
        .unwrap();
    assert_eq!(
        next.turns
            .iter()
            .map(|turn| turn.turn_id.as_str())
            .collect::<Vec<_>>(),
        ["turn-3", "turn-2", "turn-1"]
    );
    assert_eq!(next.next_before_cursor, None);

    let carry = ledger.projection_window_carry("conversation", 91).unwrap();
    assert!(carry.events.is_empty());
    assert_eq!(carry.last_turn.unwrap().turn_id, "turn-3");
    let carry = ledger.projection_window_carry("conversation", 131).unwrap();
    assert_eq!(carry.events.len(), 10);
    assert!(
        carry
            .events
            .iter()
            .all(|event| event.payload["turnId"] == "turn-5" && event.cursor < 131)
    );
    assert_eq!(carry.last_turn.unwrap().turn_id, "turn-5");
}

#[test]
fn the_carry_holds_a_settled_turn_whose_background_work_is_still_open() {
    let ledger = SqliteLedger::in_memory().unwrap();
    seed(&ledger, 5, Some(2));
    let carry = ledger.projection_window_carry("conversation", 92).unwrap();
    assert_eq!(carry.events.len(), 31);
    assert!(
        carry
            .events
            .iter()
            .all(|event| event.payload["turnId"] == "turn-2" && event.cursor < 92)
    );
}
