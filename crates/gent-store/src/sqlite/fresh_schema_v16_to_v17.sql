UPDATE gent_schema SET identity = 'gent-fresh-schema-v17' WHERE singleton = 1;
CREATE TABLE IF NOT EXISTS agent_chat_projection_events (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL REFERENCES agent_chat_conversations(conversation_id),
    source_event_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_chat_projection_events_by_conversation_cursor ON agent_chat_projection_events (conversation_id, cursor);
CREATE TRIGGER IF NOT EXISTS agent_chat_projection_from_transcript
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
            'isPartial', NEW.is_partial
        )
    );
END;
CREATE TRIGGER IF NOT EXISTS agent_chat_projection_from_lifecycle
AFTER INSERT ON events
WHEN NEW.kind = 'normalizedSessionLifecycle'
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (json_extract(NEW.payload, '$.conversationId'), 'lifecycle:' || NEW.event_id, 'lifecycle', NEW.payload);
END;
CREATE TRIGGER IF NOT EXISTS agent_chat_projection_from_activity
AFTER INSERT ON events
WHEN NEW.kind = 'providerActivity'
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (json_extract(NEW.payload, '$.conversationId'), 'activity:' || NEW.event_id, 'activity', NEW.payload);
END;
