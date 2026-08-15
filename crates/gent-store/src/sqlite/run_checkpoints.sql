CREATE TABLE IF NOT EXISTS run_checkpoints (
    checkpoint_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_cursor INTEGER NOT NULL,
    state_digest_sha256 TEXT NOT NULL,
    UNIQUE (run_id, sequence)
);
