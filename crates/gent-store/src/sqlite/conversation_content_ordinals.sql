CREATE TABLE conversation_message_ordinals (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES conversation_messages(message_id),
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    UNIQUE (conversation_id, ordinal)
);
CREATE INDEX conversation_message_ordinals_by_conversation_ordinal
    ON conversation_message_ordinals (conversation_id, ordinal DESC);
INSERT INTO conversation_message_ordinals (message_id, conversation_id, ordinal)
SELECT message_id, conversation_id,
       ROW_NUMBER() OVER (PARTITION BY conversation_id ORDER BY message_id)
FROM conversation_messages;
