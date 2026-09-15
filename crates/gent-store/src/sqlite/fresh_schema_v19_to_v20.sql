CREATE TRIGGER agent_chat_projection_from_permission_activity
AFTER INSERT ON events
WHEN NEW.kind IN ('providerPermissionpending', 'providerPermissionsettled')
AND EXISTS (
    SELECT 1 FROM agent_chat_conversations
    WHERE conversation_id = json_extract(NEW.payload, '$.conversationId')
)
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (
        json_extract(NEW.payload, '$.conversationId'),
        'activity:' || NEW.event_id,
        'activity',
        json_object(
            'conversationId', json_extract(NEW.payload, '$.conversationId'),
            'runId', json_extract(NEW.payload, '$.runId'),
            'turnId', json_extract(NEW.payload, '$.turnId'),
            'activity', json_object(
                'type', CASE NEW.kind WHEN 'providerPermissionpending' THEN 'decisionPending' ELSE 'decisionSettled' END,
                'conversationId', json_extract(NEW.payload, '$.conversationId'),
                'runId', json_extract(NEW.payload, '$.runId'),
                'turnId', json_extract(NEW.payload, '$.turnId'),
                'hostEpoch', NEW.host_epoch,
                'cursor', NEW.cursor,
                'decisionId', json_extract(NEW.payload, '$.decisionId')
            )
        )
    );
END;
INSERT OR IGNORE INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
SELECT
    json_extract(payload, '$.conversationId'),
    'activity:' || event_id,
    'activity',
    json_object(
        'conversationId', json_extract(payload, '$.conversationId'),
        'runId', json_extract(payload, '$.runId'),
        'turnId', json_extract(payload, '$.turnId'),
        'activity', json_object(
            'type', CASE kind WHEN 'providerPermissionpending' THEN 'decisionPending' ELSE 'decisionSettled' END,
            'conversationId', json_extract(payload, '$.conversationId'),
            'runId', json_extract(payload, '$.runId'),
            'turnId', json_extract(payload, '$.turnId'),
            'hostEpoch', host_epoch,
            'cursor', cursor,
            'decisionId', json_extract(payload, '$.decisionId')
        )
    )
FROM events
WHERE kind IN ('providerPermissionpending', 'providerPermissionsettled')
  AND EXISTS (
      SELECT 1 FROM agent_chat_conversations
      WHERE conversation_id = json_extract(events.payload, '$.conversationId')
  );
UPDATE gent_schema SET identity = 'gent-fresh-schema-v20' WHERE singleton = 1;
