CREATE TABLE agent_chat_steer_targets (
    message_id TEXT PRIMARY KEY NOT NULL,
    receipt_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('requested', 'claimed', 'delivered', 'rejected'))
);
