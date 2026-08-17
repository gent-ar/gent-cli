CREATE TABLE IF NOT EXISTS agent_chat_transcript_events (
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    cursor INTEGER NOT NULL CHECK (cursor > 0),
    event_id TEXT NOT NULL UNIQUE,
    turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    is_partial INTEGER NOT NULL CHECK (is_partial IN (0, 1)),
    PRIMARY KEY (conversation_id, cursor)
);
