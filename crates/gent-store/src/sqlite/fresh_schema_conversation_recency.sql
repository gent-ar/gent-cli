CREATE INDEX agent_chat_transcript_events_by_turn_kind ON agent_chat_transcript_events (turn_id, kind, is_partial, cursor);
CREATE TRIGGER agent_chat_conversation_created_recency
AFTER INSERT ON agent_chat_conversations
BEGIN
    UPDATE agent_chat_conversations
    SET updated_at_unix_ms = CAST(unixepoch('subsec') * 1000 AS INTEGER)
    WHERE conversation_id = NEW.conversation_id;
END;
CREATE TRIGGER agent_chat_conversation_projection_recency
AFTER INSERT ON agent_chat_projection_events
BEGIN
    UPDATE agent_chat_conversations
    SET updated_at_unix_ms = CAST(unixepoch('subsec') * 1000 AS INTEGER)
    WHERE conversation_id = NEW.conversation_id;
END;
