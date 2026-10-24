use rusqlite::Connection;

#[test]
fn v24_upgrade_preserves_revisions_and_unset_thinking() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL);
         INSERT INTO gent_schema VALUES (1, 'gent-fresh-schema-v24');
         CREATE TABLE agent_chat_conversation_configs (
           conversation_id TEXT, revision INTEGER, system_prompt TEXT, append_system_prompt INTEGER,
           max_turns INTEGER, disallowed_tools TEXT, UNIQUE(conversation_id, revision));
         INSERT INTO agent_chat_conversation_configs VALUES ('conversation-1', 1, 'Keep context', 1, 5, '[]');"
    ).unwrap();
    super::apply(&mut connection).unwrap();
    super::apply(&mut connection).unwrap();
    let row: (String, u64, Option<bool>) = connection
        .query_row(
            "SELECT system_prompt, revision, thinking_enabled FROM agent_chat_conversation_configs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("Keep context".into(), 1, None));
    for enabled in [Some(false), Some(true), None] {
        connection
            .execute(
                "UPDATE agent_chat_conversation_configs SET thinking_enabled = ?1",
                [enabled],
            )
            .unwrap();
        let actual: Option<bool> = connection
            .query_row(
                "SELECT thinking_enabled FROM agent_chat_conversation_configs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(actual, enabled);
    }
}
