CREATE TABLE IF NOT EXISTS agent_chat_prompt_receipts (
    request_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    turn_id TEXT NOT NULL UNIQUE REFERENCES turns(turn_id),
    message_id TEXT NOT NULL UNIQUE REFERENCES conversation_messages(message_id),
    disposition TEXT NOT NULL,
    tool_source_ids_json TEXT NOT NULL DEFAULT '[]'
);
