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
            'origin', json(NEW.origin_json),
            'recordedAtUnixMs', NEW.recorded_at_unix_ms
        )
    );
END;
