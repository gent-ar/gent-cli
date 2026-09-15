DROP TABLE IF EXISTS conversation_goals;
CREATE TABLE conversation_goals (
    creation_order INTEGER PRIMARY KEY AUTOINCREMENT, goal_id TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    revision INTEGER NOT NULL CHECK (revision > 0), status TEXT NOT NULL, record_json TEXT NOT NULL
);
CREATE INDEX conversation_goals_by_conversation ON conversation_goals (conversation_id, creation_order);
CREATE INDEX conversation_goals_by_status ON conversation_goals (status);
UPDATE gent_schema SET identity = 'gent-fresh-schema-v21' WHERE singleton = 1;
