ALTER TABLE agent_chat_prompt_dispatches RENAME TO agent_chat_prompt_dispatches_v27;
DROP INDEX agent_chat_prompt_dispatches_pending;
CREATE TABLE agent_chat_prompt_dispatches (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id),
    state TEXT NOT NULL CHECK (state IN ('pending', 'claimed', 'launching', 'started', 'settled', 'unprovable')),
    coordinator_id TEXT,
    host_epoch INTEGER,
    created_rowid INTEGER NOT NULL
);
INSERT INTO agent_chat_prompt_dispatches
    (message_id, state, coordinator_id, host_epoch, created_rowid)
SELECT message_id,
    CASE state WHEN 'claimed' THEN 'unprovable' ELSE state END,
    coordinator_id, host_epoch, created_rowid
FROM agent_chat_prompt_dispatches_v27;
DROP TABLE agent_chat_prompt_dispatches_v27;
CREATE INDEX agent_chat_prompt_dispatches_pending
    ON agent_chat_prompt_dispatches (state, created_rowid);
