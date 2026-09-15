use gent_ports::{
    ConversationActivityLedger, ConversationLedger, Ledger, PendingPermissionLedger, RunRecord,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, ConversationActivityFact,
    ConversationRecord, HostEpoch, PermissionCategory, PermissionDecisionBinding,
    PermissionDecisionRequest, PermissionRequest, PermissionRequestDigest,
};
use rusqlite::Connection;

fn request(conversation_id: &str, run_id: &str) -> PermissionDecisionRequest {
    PermissionDecisionRequest {
        binding: PermissionDecisionBinding {
            decision_id: AgentChatDecisionId("0".into()),
            request_idempotency_key: format!("codex:{run_id}:0"),
            conversation_id: AgentChatConversationId(conversation_id.into()),
            run_id: AgentChatRunId(run_id.into()),
            turn_id: format!("turn-{run_id}"),
            policy_id: "policy-1".into(),
            policy_revision: 1,
            host_epoch: HostEpoch(1),
            request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
        },
        request: PermissionRequest {
            tool_name: "shell".into(),
            category: PermissionCategory::Command,
            input: None,
            child_id: None,
        },
    }
}

#[test]
fn provider_request_ids_are_scoped_to_their_conversation_run() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-1".into(),
            },
            &RunRecord {
                run_id: "run-1".into(),
                parent_run_id: None,
                provider: "codex".into(),
            },
        )
        .unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-2".into(),
            parent_run_id: Some("run-1".into()),
            provider: "codex".into(),
        })
        .unwrap();
    let first = request("conversation-1", "run-1");
    let second = request("conversation-1", "run-2");

    ledger.save_pending_permission(&first).unwrap();
    ledger.save_pending_permission(&second).unwrap();

    assert_eq!(
        ledger
            .pending_permission(&first.binding.conversation_id, &first.binding.run_id)
            .unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        ledger
            .pending_permission(&second.binding.conversation_id, &second.binding.run_id)
            .unwrap(),
        Some(second.clone())
    );
    ledger.settle_pending_permission(&first.binding).unwrap();
    assert!(
        ledger
            .pending_permission(&second.binding.conversation_id, &second.binding.run_id)
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        ledger
            .read_conversation_activity_page("conversation-1", "run-2", 0, 8)
            .unwrap()
            .facts
            .as_slice(),
        [ConversationActivityFact::DecisionPending { .. }]
    ));
}

#[test]
fn v17_pending_requests_survive_the_composite_identity_migration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
             INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v17'); \
             CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE events (cursor INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
             CREATE TABLE agent_chat_projection_events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL, source_event_id TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, payload TEXT NOT NULL); \
             CREATE TABLE pending_provider_permissions (decision_id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT NOT NULL, run_id TEXT NOT NULL, binding_json TEXT NOT NULL, request_json TEXT NOT NULL, UNIQUE(conversation_id, run_id)); \
             INSERT INTO conversations VALUES ('conversation-1'); \
             INSERT INTO runs VALUES ('run-1'); \
             INSERT INTO pending_provider_permissions VALUES ('0', 'conversation-1', 'run-1', '{}', '{\"request\":\"kept\"}');",
        )
        .unwrap();
    drop(connection);

    drop(SqliteLedger::open(&path).unwrap());

    let connection = Connection::open(path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT identity FROM gent_schema WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "gent-fresh-schema-v23"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT request_json FROM pending_provider_permissions",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "{\"request\":\"kept\"}"
    );
    let primary_key: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('pending_provider_permissions') WHERE pk > 0 ORDER BY pk")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(primary_key, ["conversation_id", "run_id", "decision_id"]);
}
