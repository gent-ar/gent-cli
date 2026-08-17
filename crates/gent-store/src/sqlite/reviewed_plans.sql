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
CREATE TABLE reviewed_plan_current (
    plan_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL,
    FOREIGN KEY (plan_id, revision) REFERENCES reviewed_plan_artifacts(plan_id, revision)
);
CREATE TABLE reviewed_plan_approval_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL REFERENCES receipts(idempotency_key),
    plan_id TEXT NOT NULL,
    plan_revision INTEGER NOT NULL,
    parent_run_id TEXT NOT NULL REFERENCES runs(run_id),
    implementation_run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id),
    context_policy TEXT NOT NULL,
    context_through_ordinal INTEGER NOT NULL,
    policy_workspace_id TEXT NOT NULL,
    policy_revision INTEGER NOT NULL,
    FOREIGN KEY (plan_id, plan_revision) REFERENCES reviewed_plan_artifacts(plan_id, revision)
);
