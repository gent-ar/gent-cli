CREATE TABLE IF NOT EXISTS agent_chat_projection_turns (
    conversation_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    first_projection_cursor INTEGER NOT NULL,
    last_projection_cursor INTEGER NOT NULL,
    first_transcript_cursor INTEGER,
    first_activity_cursor INTEGER,
    PRIMARY KEY (conversation_id, turn_id)
);
CREATE INDEX IF NOT EXISTS agent_chat_projection_turns_by_first ON agent_chat_projection_turns (conversation_id, first_projection_cursor);
CREATE INDEX IF NOT EXISTS agent_chat_projection_turns_by_last ON agent_chat_projection_turns (conversation_id, last_projection_cursor);
CREATE INDEX IF NOT EXISTS agent_chat_projection_events_by_turn ON agent_chat_projection_events (conversation_id, json_extract(payload, '$.turnId'), cursor);
CREATE INDEX IF NOT EXISTS agent_chat_projection_events_by_kind ON agent_chat_projection_events (conversation_id, kind, cursor);
CREATE INDEX IF NOT EXISTS agent_chat_projection_events_by_activity_type ON agent_chat_projection_events (conversation_id, json_extract(payload, '$.activity.type'), cursor) WHERE kind = 'activity';
CREATE INDEX IF NOT EXISTS turns_open_by_conversation ON turns (conversation_id) WHERE phase NOT IN ('completed', 'interrupted', 'cancelled', 'failed');
CREATE INDEX IF NOT EXISTS runs_by_conversation ON runs (conversation_id);
CREATE TRIGGER IF NOT EXISTS agent_chat_projection_turn_boundary
AFTER INSERT ON agent_chat_projection_events
WHEN EXISTS (SELECT 1 FROM turns WHERE turn_id = json_extract(NEW.payload, '$.turnId'))
BEGIN
    INSERT INTO agent_chat_projection_turns (conversation_id, turn_id, run_id, first_projection_cursor, last_projection_cursor, first_transcript_cursor, first_activity_cursor)
    SELECT NEW.conversation_id, t.turn_id, t.run_id, NEW.cursor, NEW.cursor,
        CASE WHEN NEW.kind = 'transcript' THEN json_extract(NEW.payload, '$.cursor') END,
        CASE WHEN NEW.kind = 'activity' THEN json_extract(NEW.payload, '$.activity.cursor') END
    FROM turns t WHERE t.turn_id = json_extract(NEW.payload, '$.turnId')
    ON CONFLICT (conversation_id, turn_id) DO UPDATE SET
        last_projection_cursor = excluded.last_projection_cursor,
        first_transcript_cursor = coalesce(first_transcript_cursor, excluded.first_transcript_cursor),
        first_activity_cursor = coalesce(first_activity_cursor, excluded.first_activity_cursor);
END;
INSERT OR IGNORE INTO agent_chat_projection_turns (conversation_id, turn_id, run_id, first_projection_cursor, last_projection_cursor, first_transcript_cursor, first_activity_cursor)
SELECT e.conversation_id, t.turn_id, t.run_id, MIN(e.cursor), MAX(e.cursor),
    MIN(CASE WHEN e.kind = 'transcript' THEN json_extract(e.payload, '$.cursor') END),
    MIN(CASE WHEN e.kind = 'activity' THEN json_extract(e.payload, '$.activity.cursor') END)
FROM agent_chat_projection_events e JOIN turns t ON t.turn_id = json_extract(e.payload, '$.turnId')
GROUP BY e.conversation_id, t.turn_id;
