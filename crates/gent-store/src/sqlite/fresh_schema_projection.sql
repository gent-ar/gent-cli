CREATE TABLE agent_chat_projection_events (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    source_event_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX agent_chat_projection_events_by_conversation_cursor ON agent_chat_projection_events (conversation_id, cursor);
CREATE TRIGGER agent_chat_projection_from_transcript
AFTER INSERT ON agent_chat_transcript_events
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (
        NEW.conversation_id,
        'transcript:' || NEW.event_id,
        'transcript',
        json_object(
            'cursor', NEW.cursor,
            'eventId', NEW.event_id,
            'turnId', NEW.turn_id,
            'runId', NEW.run_id,
            'kind', NEW.kind,
            'text', NEW.text,
            'isPartial', NEW.is_partial,
            'origin', json(NEW.origin_json)
        )
    );
END;
CREATE TRIGGER agent_chat_projection_from_lifecycle
AFTER INSERT ON events
WHEN NEW.kind = 'normalizedSessionLifecycle'
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (json_extract(NEW.payload, '$.conversationId'), 'lifecycle:' || NEW.event_id, 'lifecycle', NEW.payload);
END;
CREATE TRIGGER agent_chat_projection_from_activity
AFTER INSERT ON events
WHEN NEW.kind = 'providerActivity'
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (json_extract(NEW.payload, '$.conversationId'), 'activity:' || NEW.event_id, 'activity', json_set(NEW.payload, '$.activity.cursor', NEW.cursor));
END;
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
