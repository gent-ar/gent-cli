CREATE TABLE IF NOT EXISTS conversation_activity_projection_journal (
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    host_epoch INTEGER NOT NULL,
    cursor INTEGER NOT NULL CHECK (cursor > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    activity_sequence INTEGER NOT NULL CHECK (activity_sequence > 0),
    payload TEXT NOT NULL,
    PRIMARY KEY (conversation_id, run_id, cursor)
);
