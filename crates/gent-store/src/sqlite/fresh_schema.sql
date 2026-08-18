CREATE TABLE gent_schema (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    identity TEXT NOT NULL
);
INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v1');
CREATE TABLE host_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    epoch INTEGER NOT NULL,
    ingress TEXT NOT NULL DEFAULT 'open'
);
INSERT INTO host_state (singleton, epoch, ingress) VALUES (1, 1, 'open');
CREATE TABLE receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL,
    receipt_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    host_epoch INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload_digest TEXT NOT NULL
);
CREATE TABLE decisions (
    decision_id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    phase TEXT NOT NULL
);
CREATE TABLE events (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    receipt_id TEXT NOT NULL,
    host_epoch INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE TABLE event_snapshots (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    cursor INTEGER NOT NULL,
    host_epoch INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY NOT NULL);
CREATE TABLE runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    parent_run_id TEXT REFERENCES runs(run_id),
    provider TEXT NOT NULL,
    conversation_id TEXT REFERENCES conversations(conversation_id)
);
CREATE TABLE run_version_locks (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    provider TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    file_identity TEXT NOT NULL,
    digest_sha256 TEXT NOT NULL,
    version TEXT NOT NULL,
    compatibility_entry TEXT NOT NULL
);
CREATE TABLE run_leases (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    coordinator_id TEXT NOT NULL,
    host_epoch INTEGER NOT NULL
);
CREATE TABLE worktree_leases (
    worktree_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    lease_token TEXT NOT NULL UNIQUE,
    host_epoch INTEGER NOT NULL
);
CREATE TABLE run_session_bindings (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    provider_session_id TEXT NOT NULL
);
CREATE TABLE run_projections (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    host_epoch INTEGER NOT NULL,
    cursor INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE TABLE turns (
    turn_id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    phase TEXT NOT NULL,
    UNIQUE (run_id, sequence)
);
CREATE INDEX turns_by_run_sequence ON turns (run_id, sequence);
CREATE TABLE conversation_artifacts (
    artifact_id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    kind TEXT NOT NULL,
    source_turn_ids TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_version TEXT NOT NULL,
    input_digest TEXT NOT NULL,
    status TEXT NOT NULL,
    text TEXT,
    supersedes_artifact_id TEXT REFERENCES conversation_artifacts(artifact_id)
);
CREATE INDEX conversation_artifacts_by_conversation ON conversation_artifacts (conversation_id);
CREATE TABLE capability_catalog (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    capabilities TEXT NOT NULL
);
CREATE TABLE workspaces (workspace_id TEXT PRIMARY KEY NOT NULL, canonical_path TEXT NOT NULL UNIQUE);
CREATE TABLE repositories (
    repository_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    canonical_path TEXT NOT NULL UNIQUE
);
CREATE TABLE worktrees (
    worktree_id TEXT PRIMARY KEY NOT NULL,
    repository_id TEXT NOT NULL REFERENCES repositories(repository_id),
    canonical_path TEXT NOT NULL UNIQUE
);
CREATE TABLE policies (
    policy_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    scope TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    allowed_tools TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'default',
    allowed_categories TEXT NOT NULL DEFAULT '[]',
    UNIQUE (workspace_id, scope, revision)
);
CREATE TABLE git_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    worktree_id TEXT NOT NULL REFERENCES worktrees(worktree_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    kind TEXT NOT NULL,
    phase TEXT NOT NULL
);
CREATE TABLE tool_sources (
    tool_source_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    kind TEXT NOT NULL,
    source_name TEXT NOT NULL,
    declared_tools TEXT NOT NULL,
    UNIQUE (workspace_id, source_name)
);
CREATE TABLE run_checkpoints (
    checkpoint_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_cursor INTEGER NOT NULL,
    state_digest_sha256 TEXT NOT NULL,
    UNIQUE (run_id, sequence)
);
CREATE TABLE attachments (
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
CREATE INDEX attachments_by_digest ON attachments(digest_sha256);
CREATE TABLE turn_attachments (
    turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    attachment_id TEXT NOT NULL REFERENCES attachments(attachment_id),
    PRIMARY KEY (turn_id, attachment_id)
);
CREATE TABLE mcp_connectors (
    connector_id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),
    tool_source_id TEXT NOT NULL REFERENCES tool_sources(tool_source_id),
    phase TEXT NOT NULL
);
CREATE TABLE mcp_connector_leases (
    tool_source_id TEXT PRIMARY KEY NOT NULL REFERENCES tool_sources(tool_source_id),
    lease_token TEXT NOT NULL UNIQUE,
    host_epoch INTEGER NOT NULL
);
CREATE TABLE conversation_messages (
    message_id TEXT PRIMARY KEY NOT NULL,
    turn_id TEXT NOT NULL UNIQUE REFERENCES turns(turn_id),
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    text TEXT NOT NULL,
    text_digest_sha256 TEXT NOT NULL,
    byte_len INTEGER NOT NULL CHECK (byte_len > 0)
);
CREATE INDEX conversation_messages_by_run ON conversation_messages (run_id);
CREATE TABLE conversation_message_ordinals (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id),
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    UNIQUE (conversation_id, ordinal)
);
CREATE INDEX conversation_message_ordinals_by_conversation_ordinal ON conversation_message_ordinals (conversation_id, ordinal DESC);
CREATE TABLE conversation_activity_projection_journal (
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    host_epoch INTEGER NOT NULL,
    cursor INTEGER NOT NULL CHECK (cursor > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    activity_sequence INTEGER NOT NULL CHECK (activity_sequence > 0),
    payload TEXT NOT NULL,
    PRIMARY KEY (conversation_id, run_id, cursor)
);
CREATE TABLE runtime_update_journal (
    attempt_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    artifact_digest_sha256 TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (attempt_id, revision)
);
CREATE TABLE agent_chat_conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations(conversation_id),
    root_run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    effort TEXT NOT NULL,
    mode TEXT NOT NULL
);
CREATE TABLE agent_chat_run_selections (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    effort TEXT NOT NULL,
    mode TEXT NOT NULL
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
    plan_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    source_run_id TEXT NOT NULL REFERENCES runs(run_id),
    source_turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    content_digest_sha256 TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    status TEXT NOT NULL,
    PRIMARY KEY (plan_id, revision)
);
CREATE TABLE reviewed_plan_current (plan_id TEXT PRIMARY KEY NOT NULL, revision INTEGER NOT NULL,
    FOREIGN KEY (plan_id, revision) REFERENCES reviewed_plan_artifacts(plan_id, revision));
CREATE TABLE reviewed_plan_approval_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key), plan_id TEXT NOT NULL,
    plan_revision INTEGER NOT NULL,
    parent_run_id TEXT NOT NULL REFERENCES runs(run_id),
    implementation_run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id),
    context_policy TEXT NOT NULL,
    context_through_ordinal INTEGER NOT NULL,
    policy_workspace_id TEXT NOT NULL,
    policy_revision INTEGER NOT NULL,
    FOREIGN KEY (plan_id, plan_revision) REFERENCES reviewed_plan_artifacts(plan_id, revision)
);
CREATE TABLE conversation_goals (
    creation_order INTEGER PRIMARY KEY AUTOINCREMENT,
    goal_id TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    schema_version INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'completed', 'abandoned', 'failed')),
    summary TEXT NOT NULL
); CREATE INDEX conversation_goals_by_conversation ON conversation_goals (conversation_id, creation_order);
CREATE TABLE orchestration_graphs (graph_id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id), root_run_id TEXT NOT NULL REFERENCES runs(run_id), revision INTEGER NOT NULL CHECK (revision > 0), host_epoch INTEGER NOT NULL, graph_json TEXT NOT NULL); CREATE TABLE orchestration_idempotency (idempotency_key TEXT PRIMARY KEY NOT NULL, graph_id TEXT NOT NULL REFERENCES orchestration_graphs(graph_id), command_json TEXT NOT NULL);
CREATE TABLE agent_chat_transcript_events (
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id), cursor INTEGER NOT NULL CHECK (cursor > 0),
    event_id TEXT NOT NULL UNIQUE,
    turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    is_partial INTEGER NOT NULL CHECK (is_partial IN (0, 1)),
    PRIMARY KEY (conversation_id, cursor)
);
CREATE TABLE agent_chat_prompt_dispatches (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id), state TEXT NOT NULL CHECK (state IN ('pending', 'claimed', 'launching', 'started', 'settled', 'unprovable')),
    coordinator_id TEXT,
    host_epoch INTEGER,
    created_rowid INTEGER NOT NULL
);
CREATE INDEX agent_chat_prompt_dispatches_pending ON agent_chat_prompt_dispatches(state, created_rowid);
