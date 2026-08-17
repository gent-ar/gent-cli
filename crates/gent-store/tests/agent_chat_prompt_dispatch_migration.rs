use rusqlite::Connection;

const MIGRATION: &str = include_str!("../src/sqlite/agent_chat_prompt_dispatch_recovery.sql");

#[test]
fn legacy_claims_become_unprovable_during_dispatch_state_upgrade() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE conversation_messages (message_id TEXT PRIMARY KEY NOT NULL);
            CREATE TABLE agent_chat_prompt_dispatches (
                message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id),
                state TEXT NOT NULL CHECK (state IN ('pending', 'claimed', 'settled')),
                coordinator_id TEXT,
                host_epoch INTEGER,
                created_rowid INTEGER NOT NULL
            );
            CREATE INDEX agent_chat_prompt_dispatches_pending
                ON agent_chat_prompt_dispatches (state, created_rowid);
            INSERT INTO conversation_messages VALUES ('pending'), ('claimed'), ('settled');
            INSERT INTO agent_chat_prompt_dispatches VALUES
                ('pending', 'pending', NULL, NULL, 1),
                ('claimed', 'claimed', 'old-daemon', 1, 2),
                ('settled', 'settled', 'old-daemon', 1, 3);
            ",
        )
        .unwrap();
    connection.execute_batch(MIGRATION).unwrap();
    let states: Vec<(String, String)> = connection
        .prepare(
            "SELECT message_id, state FROM agent_chat_prompt_dispatches ORDER BY created_rowid",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        states,
        [
            ("pending".into(), "pending".into()),
            ("claimed".into(), "unprovable".into()),
            ("settled".into(), "settled".into()),
        ]
    );
}
