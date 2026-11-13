CREATE TABLE IF NOT EXISTS archived_conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations(conversation_id),
    receipt_id TEXT NOT NULL,
    archived_at_unix_ms INTEGER NOT NULL
);
