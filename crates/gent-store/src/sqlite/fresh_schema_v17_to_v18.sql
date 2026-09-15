CREATE TABLE pending_provider_permissions_v18 (
    decision_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    binding_json TEXT NOT NULL,
    request_json TEXT NOT NULL,
    PRIMARY KEY(conversation_id, run_id, decision_id),
    UNIQUE(conversation_id, run_id)
);
INSERT INTO pending_provider_permissions_v18 (decision_id, conversation_id, run_id, binding_json, request_json)
SELECT decision_id, conversation_id, run_id, binding_json, request_json FROM pending_provider_permissions;
DROP TABLE pending_provider_permissions;
ALTER TABLE pending_provider_permissions_v18 RENAME TO pending_provider_permissions;
UPDATE gent_schema SET identity = 'gent-fresh-schema-v18' WHERE singleton = 1;
