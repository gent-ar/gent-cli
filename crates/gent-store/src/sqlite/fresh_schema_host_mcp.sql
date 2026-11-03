CREATE TABLE IF NOT EXISTS host_mcp_operations (
    generation TEXT NOT NULL,
    request_key TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    turn_id TEXT NOT NULL REFERENCES turns(turn_id),
    coordinator_id TEXT NOT NULL,
    host_epoch INTEGER NOT NULL,
    policy_id TEXT NOT NULL REFERENCES policies(policy_id),
    source_sha256 TEXT NOT NULL,
    request_sha256 TEXT NOT NULL,
    binding_sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','completed','unknown')),
    response BLOB,
    CHECK((state = 'completed' AND response IS NOT NULL) OR (state != 'completed' AND response IS NULL)),
    PRIMARY KEY(generation, request_key)
);
CREATE INDEX IF NOT EXISTS host_mcp_by_turn ON host_mcp_operations(turn_id);
