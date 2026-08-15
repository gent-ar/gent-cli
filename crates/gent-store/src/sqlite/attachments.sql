CREATE TABLE IF NOT EXISTS attachments (
    attachment_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    receipt_id TEXT NOT NULL UNIQUE,
    host_epoch INTEGER NOT NULL,
    state TEXT NOT NULL,
    received_bytes INTEGER NOT NULL,
    display_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    digest_sha256 TEXT NOT NULL UNIQUE,
    storage_key TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS turn_attachments (
    turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    attachment_id TEXT NOT NULL REFERENCES attachments(attachment_id),
    PRIMARY KEY (turn_id, attachment_id)
);
