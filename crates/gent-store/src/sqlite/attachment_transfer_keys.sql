CREATE TABLE attachments_v14 (
    attachment_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    receipt_id TEXT NOT NULL UNIQUE,
    host_epoch INTEGER NOT NULL,
    state TEXT NOT NULL,
    received_bytes INTEGER NOT NULL,
    display_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    digest_sha256 TEXT NOT NULL,
    storage_key TEXT NOT NULL,
    staging_key TEXT NOT NULL UNIQUE
);
INSERT INTO attachments_v14
SELECT attachment_id, idempotency_key, receipt_id, host_epoch, state, received_bytes,
       display_name, media_type, byte_len, digest_sha256, storage_key, storage_key
FROM attachments;
CREATE TABLE turn_attachments_v14 (
    turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    attachment_id TEXT NOT NULL REFERENCES attachments_v14(attachment_id),
    PRIMARY KEY (turn_id, attachment_id)
);
INSERT INTO turn_attachments_v14 SELECT turn_id, attachment_id FROM turn_attachments;
DROP TABLE turn_attachments;
DROP TABLE attachments;
ALTER TABLE attachments_v14 RENAME TO attachments;
ALTER TABLE turn_attachments_v14 RENAME TO turn_attachments;
CREATE INDEX attachments_by_digest ON attachments(digest_sha256);
