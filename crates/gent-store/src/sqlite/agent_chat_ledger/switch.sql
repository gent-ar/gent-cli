CREATE TABLE IF NOT EXISTS agent_chat_selection_switch_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    parent_run_id TEXT NOT NULL REFERENCES agent_chat_run_selections(run_id),
    run_id TEXT NOT NULL UNIQUE REFERENCES agent_chat_run_selections(run_id),
    context_policy TEXT NOT NULL CHECK (context_policy IN ('preserve', 'clear')),
    context_through_ordinal INTEGER NOT NULL CHECK (context_through_ordinal >= 0)
);
