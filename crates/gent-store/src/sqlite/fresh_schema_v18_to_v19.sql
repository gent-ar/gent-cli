DROP TRIGGER IF EXISTS agent_chat_projection_from_activity;
CREATE TRIGGER agent_chat_projection_from_activity
AFTER INSERT ON events
WHEN NEW.kind = 'providerActivity'
BEGIN
    INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload)
    VALUES (json_extract(NEW.payload, '$.conversationId'), 'activity:' || NEW.event_id, 'activity', json_set(NEW.payload, '$.activity.cursor', NEW.cursor));
END;
UPDATE agent_chat_projection_events
SET payload = json_set(
    payload,
    '$.activity.cursor',
    (SELECT cursor FROM events WHERE 'activity:' || event_id = source_event_id)
)
WHERE kind = 'activity'
  AND json_extract(payload, '$.activity.cursor') = 0
  AND EXISTS (SELECT 1 FROM events WHERE 'activity:' || event_id = source_event_id);
UPDATE gent_schema SET identity = 'gent-fresh-schema-v19' WHERE singleton = 1;
