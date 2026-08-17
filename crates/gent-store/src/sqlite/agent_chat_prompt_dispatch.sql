CREATE TABLE IF NOT EXISTS agent_chat_prompt_dispatches (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id),
    state TEXT NOT NULL CHECK (state IN ('pending', 'claimed', 'settled')),
    coordinator_id TEXT,
    host_epoch INTEGER,
    created_rowid INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_chat_prompt_dispatches_pending
    ON agent_chat_prompt_dispatches (state, created_rowid);
INSERT OR IGNORE INTO agent_chat_prompt_dispatches
    (message_id, state, coordinator_id, host_epoch, created_rowid)
SELECT message_id, 'pending', NULL, NULL, rowid
FROM agent_chat_prompt_receipts
WHERE disposition = 'send';
