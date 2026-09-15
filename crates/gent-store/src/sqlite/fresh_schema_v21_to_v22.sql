ALTER TABLE agent_chat_transcript_events ADD COLUMN origin_json TEXT;
UPDATE agent_chat_transcript_events SET origin_json = '{"kind":"user"}' WHERE kind = 'userMessage' AND event_id LIKE 'user:%';
UPDATE agent_chat_projection_events SET payload = json_set(payload, '$.origin', json('{"kind":"user"}'))
WHERE kind = 'transcript' AND json_extract(payload, '$.kind') = 'userMessage' AND json_extract(payload, '$.eventId') LIKE 'user:%';
DROP TRIGGER IF EXISTS agent_chat_projection_from_transcript;
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
