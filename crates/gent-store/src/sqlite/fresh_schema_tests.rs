use rusqlite::Connection;

use super::{apply, has_table};

#[test]
fn rejects_an_existing_ledger_without_altering_data() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE old_gent_ledger (identity TEXT PRIMARY KEY);")
        .unwrap();
    assert!(apply(&mut connection).is_err());
}

#[test]
fn rejects_an_unknown_fresh_schema_identity() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v4');",
        )
        .unwrap();
    assert!(apply(&mut connection).is_err());
}

#[test]
fn current_fresh_schema_reopens_without_running_its_migration_again() {
    let mut connection = Connection::open_in_memory().unwrap();
    apply(&mut connection).unwrap();
    apply(&mut connection).unwrap();
    let identity: String = connection
        .query_row(
            "SELECT identity FROM gent_schema WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(identity, "gent-fresh-schema-v24");
    assert!(has_table(&connection, "agent_chat_projection_events").unwrap());
}

#[test]
fn migrates_the_previous_fresh_schema_identity() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v10'); \
             CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE workspaces (workspace_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE receipts (idempotency_key TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE mcp_connectors (connector_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE tool_sources (tool_source_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL, root_run_id TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL, effort TEXT NOT NULL, mode TEXT NOT NULL, workspace_id TEXT); \
             CREATE TABLE agent_chat_run_selections (run_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_prompt_receipts (request_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL, conversation_id TEXT NOT NULL, run_id TEXT NOT NULL, turn_id TEXT NOT NULL, message_id TEXT NOT NULL, disposition TEXT NOT NULL); \
             CREATE TABLE agent_chat_transcript_events (conversation_id TEXT NOT NULL, cursor INTEGER NOT NULL, event_id TEXT NOT NULL UNIQUE, turn_id TEXT NOT NULL, run_id TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, is_partial INTEGER NOT NULL, PRIMARY KEY (conversation_id, cursor)); \
             CREATE TABLE events (cursor INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL);",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    let identity: String = connection
        .query_row(
            "SELECT identity FROM gent_schema WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(identity, "gent-fresh-schema-v24");
    let column: String = connection
        .query_row(
            "SELECT name FROM pragma_table_info('agent_chat_prompt_receipts') WHERE name = 'tool_source_ids_json'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(column, "tool_source_ids_json");
}

#[test]
fn replaces_removed_permission_modes_during_schema_migration() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v15'); \
             CREATE TABLE policies (mode TEXT NOT NULL); \
             CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE pending_provider_permissions (decision_id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT NOT NULL, run_id TEXT NOT NULL, binding_json TEXT NOT NULL, request_json TEXT NOT NULL, UNIQUE(conversation_id, run_id)); \
             CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_transcript_events (conversation_id TEXT NOT NULL, cursor INTEGER NOT NULL, event_id TEXT NOT NULL UNIQUE, turn_id TEXT NOT NULL, run_id TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, is_partial INTEGER NOT NULL, PRIMARY KEY (conversation_id, cursor)); \
             CREATE TABLE events (cursor INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             INSERT INTO policies (mode) VALUES ('default'), ('plan'), ('autonomous');",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    let modes: Vec<String> = connection
        .prepare("SELECT mode FROM policies ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(modes, ["askEveryTime", "askEveryTime", "autonomous"]);
}

#[test]
fn journals_transcript_lifecycle_and_activity_in_one_cursor_order() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v16'); \
             CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE pending_provider_permissions (decision_id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT NOT NULL, run_id TEXT NOT NULL, binding_json TEXT NOT NULL, request_json TEXT NOT NULL, UNIQUE(conversation_id, run_id)); \
             CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_transcript_events (conversation_id TEXT NOT NULL, cursor INTEGER NOT NULL, event_id TEXT NOT NULL UNIQUE, turn_id TEXT NOT NULL, run_id TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, is_partial INTEGER NOT NULL, PRIMARY KEY (conversation_id, cursor)); \
             CREATE TABLE events (cursor INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             INSERT INTO agent_chat_conversations (conversation_id) VALUES ('conversation-1');",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    connection
        .execute_batch(
            "INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial) VALUES ('conversation-1', 1, 'transcript-1', 'turn-1', 'run-1', 'assistantMessage', 'hello', 0); \
             INSERT INTO events (cursor, event_id, receipt_id, host_epoch, kind, payload) VALUES (1, 'lifecycle-1', 'receipt-1', 1, 'normalizedSessionLifecycle', '{\"conversationId\":\"conversation-1\"}'); \
             INSERT INTO events (cursor, event_id, receipt_id, host_epoch, kind, payload) VALUES (2, 'activity-1', 'receipt-1', 1, 'providerActivity', '{\"conversationId\":\"conversation-1\",\"activity\":{\"cursor\":0}}'); \
             INSERT INTO events (cursor, event_id, receipt_id, host_epoch, kind, payload) VALUES (3, 'permission-1', 'receipt-1', 4, 'providerPermissionpending', '{\"conversationId\":\"conversation-1\",\"runId\":\"run-1\",\"turnId\":\"turn-1\",\"decisionId\":\"decision-1\"}');",
        )
        .unwrap();
    let rows: Vec<(u64, String, Option<u64>, Option<String>, Option<String>)> = connection
        .prepare("SELECT cursor, kind, json_extract(payload, '$.activity.cursor'), json_extract(payload, '$.activity.type'), json_extract(payload, '$.activity.decisionId') FROM agent_chat_projection_events ORDER BY cursor")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        [
            (1, "transcript".into(), None, None, None),
            (2, "lifecycle".into(), None, None, None),
            (3, "activity".into(), Some(2), None, None),
            (
                4,
                "activity".into(),
                Some(3),
                Some("decisionPending".into()),
                Some("decision-1".into())
            )
        ]
    );
}

#[test]
fn migrates_existing_activity_projection_cursors() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v18'); \
             CREATE TABLE events (cursor INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_projection_events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL, source_event_id TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             INSERT INTO events (cursor, event_id, receipt_id, host_epoch, kind, payload) VALUES (7, 'activity-1', 'receipt-1', 1, 'providerActivity', '{\"conversationId\":\"conversation-1\",\"activity\":{\"cursor\":0}}'); \
             INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES ('conversation-1', 'activity:activity-1', 'activity', '{\"conversationId\":\"conversation-1\",\"activity\":{\"cursor\":0}}');",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    let (identity, cursor): (String, u64) = connection
        .query_row(
            "SELECT (SELECT identity FROM gent_schema WHERE singleton = 1), json_extract(payload, '$.activity.cursor') FROM agent_chat_projection_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((identity.as_str(), cursor), ("gent-fresh-schema-v24", 7));
}

#[test]
fn migrates_permission_decisions_into_the_projection() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v19'); \
             CREATE TABLE events (cursor INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_projection_events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL, source_event_id TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             INSERT INTO agent_chat_conversations VALUES ('conversation-1'); \
             INSERT INTO events (cursor, event_id, receipt_id, host_epoch, kind, payload) VALUES (9, 'permission-1', 'receipt-1', 6, 'providerPermissionpending', '{\"conversationId\":\"conversation-1\",\"runId\":\"run-1\",\"turnId\":\"turn-1\",\"decisionId\":\"decision-1\"}');",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    let values: (String, String, String, u64) = connection
        .query_row(
            "SELECT (SELECT identity FROM gent_schema WHERE singleton = 1), json_extract(payload, '$.activity.type'), json_extract(payload, '$.activity.decisionId'), json_extract(payload, '$.activity.cursor') FROM agent_chat_projection_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        values,
        (
            "gent-fresh-schema-v24".into(),
            "decisionPending".into(),
            "decision-1".into(),
            9
        )
    );
}

#[test]
fn migrates_transcript_prompts_to_a_typed_user_origin() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v21'); \
             CREATE TABLE agent_chat_transcript_events (conversation_id TEXT NOT NULL, cursor INTEGER NOT NULL, event_id TEXT NOT NULL UNIQUE, turn_id TEXT NOT NULL, run_id TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, is_partial INTEGER NOT NULL, PRIMARY KEY (conversation_id, cursor)); \
             CREATE TABLE agent_chat_projection_events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL, source_event_id TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             INSERT INTO agent_chat_transcript_events VALUES ('conversation-1', 1, 'user:message-1', 'turn-1', 'run-1', 'userMessage', 'hello', 0); \
             INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES ('conversation-1', 'transcript:user:message-1', 'transcript', '{\"eventId\":\"user:message-1\",\"kind\":\"userMessage\"}');",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    connection
        .execute(
            "INSERT INTO agent_chat_transcript_events VALUES ('conversation-1', 2, 'assistant-1', 'turn-1', 'run-1', 'assistantMessage', 'hi', 0, NULL)",
            [],
        )
        .unwrap();
    let origins = connection
        .prepare("SELECT json_extract(payload, '$.origin.kind'), (SELECT identity FROM gent_schema) FROM agent_chat_projection_events ORDER BY cursor")
        .unwrap()
        .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        origins,
        [
            (Some("user".into()), "gent-fresh-schema-v24".into()),
            (None, "gent-fresh-schema-v24".into()),
        ]
    );
}

#[test]
fn migrates_a_v23_ledger_into_the_conversation_link_table() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v23');",
        )
        .unwrap();
    apply(&mut connection).unwrap();
    let identity: String = connection
        .query_row(
            "SELECT identity FROM gent_schema WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(identity, "gent-fresh-schema-v24");
    assert!(has_table(&connection, "agent_chat_conversation_links").unwrap());
}
