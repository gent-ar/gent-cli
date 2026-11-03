use rusqlite::Connection;

use super::{SCHEMA, SCHEMA_ID, apply};

#[test]
fn signed_update_schema_number_matches_the_physical_schema_identity() {
    assert_eq!(
        SCHEMA_ID,
        format!(
            "gent-fresh-schema-v{}",
            crate::CURRENT_LEDGER_SCHEMA_VERSION
        )
    );
}

#[test]
fn published_v23_and_current_v26_preserve_transcript_identity_and_content() {
    for old_version in [23, 26] {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        if old_version == 23 {
            connection
                .execute_batch(
                    "DROP TABLE agent_chat_steer_targets;
                 DROP TABLE agent_chat_conversation_links;
                 ALTER TABLE agent_chat_conversation_configs DROP COLUMN thinking_enabled;
                 UPDATE gent_schema SET identity = 'gent-fresh-schema-v23';",
                )
                .unwrap();
        }
        connection.execute_batch(
            "INSERT INTO conversations VALUES ('conversation');
             INSERT INTO runs (run_id,provider,conversation_id) VALUES ('run','codex','conversation');
             INSERT INTO turns (turn_id,conversation_id,run_id,sequence,phase)
             VALUES ('turn','conversation','run',1,'completed');
             INSERT INTO agent_chat_conversations
             (conversation_id,root_run_id,provider,model,effort,mode)
             VALUES ('conversation','run','codex','fixture','default','ask');
             INSERT INTO agent_chat_transcript_events
             (conversation_id,cursor,event_id,turn_id,run_id,kind,text,is_partial,origin_json)
             VALUES ('conversation',17,'native-event','turn','run','assistant','kept verbatim',0,NULL);",
        ).unwrap();
        apply(&mut connection).unwrap();
        apply(&mut connection).unwrap();
        let row: (String, u64, String, String) = connection
            .query_row(
                "SELECT event_id,cursor,text,(SELECT identity FROM gent_schema)
             FROM agent_chat_transcript_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "native-event".into(),
                17,
                "kept verbatim".into(),
                SCHEMA_ID.into()
            )
        );
        assert!(super::has_table(&connection, "agent_chat_steer_targets").unwrap());
        assert!(
            super::has_column(
                &connection,
                "agent_chat_conversation_configs",
                "thinking_enabled"
            )
            .unwrap()
        );
    }
}
