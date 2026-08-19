CREATE TABLE IF NOT EXISTS agent_chat_conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations(conversation_id),
    root_run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    effort TEXT NOT NULL,
    mode TEXT NOT NULL,
    workspace_id TEXT REFERENCES workspaces(workspace_id)
);
CREATE TABLE IF NOT EXISTS agent_chat_run_selections (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    effort TEXT NOT NULL,
    mode TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_chat_create_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES agent_chat_run_selections(run_id)
);
