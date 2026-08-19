CREATE TABLE agent_chat_conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations(conversation_id),
    root_run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id),
    provider TEXT NOT NULL, model TEXT NOT NULL, effort TEXT NOT NULL, mode TEXT NOT NULL,
    workspace_id TEXT REFERENCES workspaces(workspace_id)
);
CREATE TABLE agent_chat_run_selections (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    provider TEXT NOT NULL, model TEXT NOT NULL, effort TEXT NOT NULL, mode TEXT NOT NULL
);
CREATE TABLE agent_chat_create_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES agent_chat_run_selections(run_id)
);
CREATE TABLE agent_chat_prompt_receipts (
    request_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    turn_id TEXT NOT NULL UNIQUE REFERENCES turns(turn_id),
    message_id TEXT NOT NULL UNIQUE REFERENCES conversation_messages(message_id),
    disposition TEXT NOT NULL
);
CREATE TABLE agent_chat_selection_switch_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    parent_run_id TEXT NOT NULL REFERENCES agent_chat_run_selections(run_id),
    run_id TEXT NOT NULL UNIQUE REFERENCES agent_chat_run_selections(run_id),
    context_policy TEXT NOT NULL CHECK (context_policy IN ('preserve', 'clear')),
    context_through_ordinal INTEGER NOT NULL CHECK (context_through_ordinal >= 0)
);
CREATE TABLE reviewed_plan_artifacts (
    plan_id TEXT NOT NULL, revision INTEGER NOT NULL CHECK (revision > 0),
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    source_run_id TEXT NOT NULL REFERENCES runs(run_id),
    source_turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    content_digest_sha256 TEXT NOT NULL, artifact_json TEXT NOT NULL, status TEXT NOT NULL,
    PRIMARY KEY (plan_id, revision)
);
CREATE TABLE reviewed_plan_current (plan_id TEXT PRIMARY KEY NOT NULL, revision INTEGER NOT NULL,
    FOREIGN KEY (plan_id, revision) REFERENCES reviewed_plan_artifacts(plan_id, revision));
CREATE TABLE reviewed_plan_approval_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key), plan_id TEXT NOT NULL,
    plan_revision INTEGER NOT NULL, parent_run_id TEXT NOT NULL REFERENCES runs(run_id),
    implementation_run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id), context_policy TEXT NOT NULL,
    context_through_ordinal INTEGER NOT NULL, policy_workspace_id TEXT NOT NULL,
    policy_revision INTEGER NOT NULL,
    FOREIGN KEY (plan_id, plan_revision) REFERENCES reviewed_plan_artifacts(plan_id, revision)
);
CREATE TABLE conversation_goals (
    creation_order INTEGER PRIMARY KEY AUTOINCREMENT, goal_id TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id), schema_version INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'completed', 'abandoned', 'failed')),
    summary TEXT NOT NULL
);
CREATE INDEX conversation_goals_by_conversation ON conversation_goals (conversation_id, creation_order);
CREATE TABLE orchestration_graph_facts (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT, graph_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0), idempotency_key TEXT NOT NULL,
    kind TEXT NOT NULL, payload TEXT NOT NULL
);
CREATE INDEX orchestration_graph_facts_by_graph_cursor
    ON orchestration_graph_facts (graph_id, cursor);
CREATE TABLE orchestration_idempotency (
    idempotency_key TEXT PRIMARY KEY NOT NULL, graph_id TEXT NOT NULL, command_json TEXT NOT NULL
);
CREATE TABLE agent_chat_transcript_events (
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    cursor INTEGER NOT NULL CHECK (cursor > 0), event_id TEXT NOT NULL UNIQUE,
    turn_id TEXT NOT NULL REFERENCES turns(turn_id), run_id TEXT NOT NULL REFERENCES runs(run_id),
    kind TEXT NOT NULL, text TEXT NOT NULL, is_partial INTEGER NOT NULL CHECK (is_partial IN (0, 1)),
    PRIMARY KEY (conversation_id, cursor)
);
CREATE TABLE agent_chat_prompt_dispatches (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id),
    state TEXT NOT NULL CHECK (state IN ('awaiting_readiness', 'provisioning', 'pending', 'claimed', 'launching', 'started', 'settled', 'unprovable')),
    coordinator_id TEXT, host_epoch INTEGER, created_rowid INTEGER NOT NULL
);
CREATE INDEX agent_chat_prompt_dispatches_pending ON agent_chat_prompt_dispatches(state, created_rowid);
CREATE TABLE normalized_session_batches (
    lifecycle_event_id TEXT PRIMARY KEY NOT NULL REFERENCES events(event_id), payload TEXT NOT NULL,
    lifecycle_cursor INTEGER NOT NULL CHECK (lifecycle_cursor > 0), transcript_event_id TEXT UNIQUE,
    transcript_cursor INTEGER CHECK (transcript_cursor > 0), activity_event_id TEXT UNIQUE,
    activity_cursor INTEGER CHECK (activity_cursor > 0),
    CHECK ((transcript_event_id IS NULL) = (transcript_cursor IS NULL)),
    CHECK ((activity_event_id IS NULL) = (activity_cursor IS NULL))
);
