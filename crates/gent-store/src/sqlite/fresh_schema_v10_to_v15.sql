CREATE TABLE IF NOT EXISTS prompt_templates (creation_order INTEGER PRIMARY KEY AUTOINCREMENT, template_id TEXT NOT NULL UNIQUE, schema_version INTEGER NOT NULL, name TEXT NOT NULL, body TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pending_provider_permissions (
    decision_id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id), binding_json TEXT NOT NULL, request_json TEXT NOT NULL,
    UNIQUE(conversation_id, run_id)
);
CREATE TABLE IF NOT EXISTS forge_connectors (
    connector_id TEXT PRIMARY KEY NOT NULL REFERENCES mcp_connectors(connector_id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    tool_source_id TEXT NOT NULL REFERENCES tool_sources(tool_source_id),
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT NOT NULL,
    phase TEXT NOT NULL,
    declared_tools TEXT NOT NULL,
    discovered_tools TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
);
CREATE TABLE IF NOT EXISTS agent_chat_sessions (
    session_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_chat_session_conversations (
    session_id TEXT NOT NULL REFERENCES agent_chat_sessions(session_id),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (session_id, conversation_id),
    UNIQUE (session_id, ordinal)
);
CREATE INDEX IF NOT EXISTS agent_chat_sessions_by_workspace ON agent_chat_sessions(workspace_id, updated_at DESC);
CREATE TABLE IF NOT EXISTS agent_chat_conversation_configs (
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    revision INTEGER NOT NULL CHECK (revision > 0),
    system_prompt TEXT,
    append_system_prompt INTEGER NOT NULL DEFAULT 0,
    max_turns INTEGER,
    disallowed_tools TEXT NOT NULL DEFAULT '[]',
    UNIQUE (conversation_id, revision)
);
CREATE TABLE IF NOT EXISTS agent_chat_file_checkpoints (
    checkpoint_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    message_ordinal INTEGER NOT NULL CHECK (message_ordinal >= 0),
    created_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_chat_file_checkpoints_by_conversation ON agent_chat_file_checkpoints(conversation_id, created_at_unix_ms DESC);
CREATE TABLE IF NOT EXISTS agent_chat_checkpoint_files (
    checkpoint_id TEXT NOT NULL REFERENCES agent_chat_file_checkpoints(checkpoint_id),
    file_path TEXT NOT NULL,
    storage_key TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    PRIMARY KEY (checkpoint_id, file_path)
);
CREATE TABLE IF NOT EXISTS agent_chat_checkpoint_restore_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    checkpoint_id TEXT NOT NULL REFERENCES agent_chat_file_checkpoints(checkpoint_id),
    run_id TEXT NOT NULL UNIQUE REFERENCES agent_chat_run_selections(run_id),
    visible_through_ordinal INTEGER NOT NULL CHECK (visible_through_ordinal >= 0)
);
CREATE TABLE IF NOT EXISTS agent_chat_fork_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    source_conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    conversation_id TEXT NOT NULL UNIQUE REFERENCES agent_chat_conversations(conversation_id),
    run_id TEXT NOT NULL UNIQUE REFERENCES agent_chat_run_selections(run_id),
    fork_through_message_id TEXT NOT NULL,
    context_through_ordinal INTEGER NOT NULL CHECK (context_through_ordinal >= 0)
);
CREATE TABLE IF NOT EXISTS agent_chat_side_questions (
    side_question_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    question TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'answered', 'failed', 'cancelled')),
    answer TEXT,
    failure_reason TEXT,
    created_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_chat_side_questions_by_conversation ON agent_chat_side_questions(conversation_id, created_at_unix_ms DESC);
CREATE INDEX IF NOT EXISTS agent_chat_side_questions_by_status ON agent_chat_side_questions(status);
CREATE TABLE IF NOT EXISTS agent_chat_side_question_cancel_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    side_question_id TEXT NOT NULL REFERENCES agent_chat_side_questions(side_question_id)
);
CREATE TABLE IF NOT EXISTS automation_definitions (
    automation_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    name TEXT NOT NULL,
    working_directory TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    action TEXT NOT NULL,
    trigger TEXT NOT NULL,
    condition TEXT,
    selection TEXT NOT NULL,
    chain_target TEXT,
    notifications TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS automation_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    automation_id TEXT NOT NULL REFERENCES automation_definitions(automation_id),
    conversation_id TEXT REFERENCES conversations(conversation_id),
    parent_run_id TEXT REFERENCES automation_runs(run_id),
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    status TEXT NOT NULL,
    summary TEXT,
    error TEXT,
    condition_result INTEGER CHECK (condition_result IN (0, 1))
);
CREATE INDEX IF NOT EXISTS automation_runs_by_automation ON automation_runs(automation_id, started_at DESC);
UPDATE gent_schema SET identity = 'gent-fresh-schema-v15' WHERE singleton = 1;
