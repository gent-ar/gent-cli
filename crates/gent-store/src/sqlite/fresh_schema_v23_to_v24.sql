CREATE TABLE IF NOT EXISTS agent_chat_conversation_links (
    child_conversation_id TEXT PRIMARY KEY,
    parent_conversation_id TEXT NOT NULL,
    label TEXT NOT NULL,
    created_run_id TEXT NOT NULL,
    created_turn_id TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_chat_conversation_links_parent ON agent_chat_conversation_links (parent_conversation_id, created_at_unix_seconds);
