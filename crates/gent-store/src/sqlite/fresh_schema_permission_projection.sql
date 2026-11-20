DROP TRIGGER IF EXISTS agent_chat_projection_from_permission_activity;
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
            'activity', json_patch(
                json_object(
                    'type', CASE NEW.kind WHEN 'providerPermissionpending' THEN 'decisionPending' ELSE 'decisionSettled' END,
                    'conversationId', json_extract(NEW.payload, '$.conversationId'),
                    'runId', json_extract(NEW.payload, '$.runId'),
                    'turnId', json_extract(NEW.payload, '$.turnId'),
                    'hostEpoch', NEW.host_epoch,
                    'cursor', NEW.cursor,
                    'decisionId', json_extract(NEW.payload, '$.decisionId')
                ),
                json_object('request', json_extract(NEW.payload, '$.request'))
            )
        )
    );
END;
