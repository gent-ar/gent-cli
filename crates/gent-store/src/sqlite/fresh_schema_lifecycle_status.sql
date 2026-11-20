CREATE INDEX IF NOT EXISTS run_lifecycle_facts_root ON run_lifecycle_facts (run_id, cursor)
WHERE coalesce(json_extract(payload, '$.lifecycle.event.type'), json_extract(payload, '$.lifecycle.signal.type')) IN ('turnStarted', 'turnEnded', 'rootActivity', 'providerFailure');
CREATE INDEX IF NOT EXISTS run_lifecycle_facts_work ON run_lifecycle_facts (run_id, cursor)
WHERE coalesce(json_extract(payload, '$.lifecycle.event.type'), json_extract(payload, '$.lifecycle.signal.type')) IN ('childStarted', 'childPhase', 'childTerminal', 'commandPhase', 'commandTerminal');
