UPDATE agent_chat_projection_events AS p
SET payload = json_set(p.payload, '$.activity.outcome', json_extract(f.payload, '$.outcome'))
FROM conversation_activity_facts AS f
WHERE p.kind = 'activity'
  AND json_extract(p.payload, '$.activity.type') = 'decisionSettled'
  AND json_type(p.payload, '$.activity.outcome') IS NULL
  AND f.conversation_id = p.conversation_id
  AND f.run_id = json_extract(p.payload, '$.runId')
  AND f.cursor = json_extract(p.payload, '$.activity.cursor')
  AND json_extract(f.payload, '$.type') = 'decisionSettled'
  AND json_extract(f.payload, '$.decisionId') = json_extract(p.payload, '$.activity.decisionId')
  AND json_extract(f.payload, '$.outcome') IN ('allowed', 'denied', 'skipped', 'cancelled');
