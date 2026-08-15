CREATE TABLE IF NOT EXISTS runtime_update_journal (
    attempt_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    artifact_digest_sha256 TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (attempt_id, revision)
);
