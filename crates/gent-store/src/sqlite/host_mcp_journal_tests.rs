use super::*;
use gent_types::HostEpoch;

#[test]
fn host_mcp_schema_v26_and_v27_upgrade_without_resetting_existing_turns() {
    for previous in ["gent-fresh-schema-v26", "gent-fresh-schema-v27"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ledger.db");
        let ledger = SqliteLedger::open(&path).unwrap();
        let scope = seed(&ledger);
        ledger
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE host_mcp_operations;")
            .unwrap();
        ledger
            .lock()
            .unwrap()
            .execute("UPDATE gent_schema SET identity=?1", [previous])
            .unwrap();
        drop(ledger);
        let ledger = SqliteLedger::open(&path).unwrap();
        assert_eq!(
            ledger
                .reserve_host_mcp(&scope, "route:1", b"request")
                .unwrap(),
            HostMcpAdmission::Fresh
        );
        let identity: String = ledger
            .lock()
            .unwrap()
            .query_row("SELECT identity FROM gent_schema", [], |r| r.get(0))
            .unwrap();
        assert_eq!(identity, "gent-fresh-schema-v30");
    }
}

#[test]
fn host_mcp_schema_v27_with_existing_completed_operation_preserves_result() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ledger.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let scope = seed(&ledger);
    ledger.reserve_host_mcp(&scope, "key", b"request").unwrap();
    ledger.complete_host_mcp(&scope, "key", b"result").unwrap();
    ledger
        .lock()
        .unwrap()
        .execute(
            "UPDATE gent_schema SET identity='gent-fresh-schema-v27'",
            [],
        )
        .unwrap();
    drop(ledger);
    let ledger = SqliteLedger::open(&path).unwrap();
    assert_eq!(
        ledger.reserve_host_mcp(&scope, "key", b"request").unwrap(),
        HostMcpAdmission::Completed(b"result".to_vec())
    );
}

fn seed(ledger: &SqliteLedger) -> HostMcpScope {
    let c = ledger.lock().unwrap();
    c.execute_batch("INSERT INTO conversations VALUES('conversation');
        INSERT INTO runs(run_id,provider,conversation_id) VALUES('run','claude','conversation');
        INSERT INTO workspaces(workspace_id,canonical_path) VALUES('workspace','C:/fixture');
        INSERT INTO agent_chat_conversations(conversation_id,root_run_id,provider,model,effort,mode,workspace_id) VALUES('conversation','run','claude','default','default','autonomous','workspace');
        INSERT INTO turns VALUES('turn','conversation','run',1,'active');
        INSERT INTO run_leases VALUES('run','owner',1);
        INSERT INTO policies(policy_id,workspace_id,scope,revision,allowed_tools) VALUES('policy','workspace','providerPermissions',1,'[]');").unwrap();
    HostMcpScope {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        coordinator_id: "owner".into(),
        host_epoch: HostEpoch(1),
        policy_id: "policy".into(),
        source_sha256: "a".repeat(64),
        generation: "b".repeat(32),
    }
}

#[test]
fn host_mcp_replay_survives_reopen_without_storing_request_secrets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ledger.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let s = seed(&ledger);
    assert_eq!(
        ledger
            .reserve_host_mcp(&s, "route:1", b"host-secret-canary")
            .unwrap(),
        HostMcpAdmission::Fresh
    );
    ledger
        .complete_host_mcp(&s, "route:1", b"{\"result\":1}")
        .unwrap();
    drop(ledger);
    let ledger = SqliteLedger::open(&path).unwrap();
    assert_eq!(
        ledger
            .reserve_host_mcp(&s, "route:1", b"host-secret-canary")
            .unwrap(),
        HostMcpAdmission::Completed(b"{\"result\":1}".to_vec())
    );
    assert!(ledger.reserve_host_mcp(&s, "route:1", b"changed").is_err());
    let dump: String = ledger
        .lock()
        .unwrap()
        .query_row("SELECT request_sha256 FROM host_mcp_operations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(dump.len(), 64);
    assert!(!dump.contains("canary"));
}

#[test]
fn host_mcp_pending_recovery_is_unknown_and_cannot_complete_after_fence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ledger.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let s = seed(&ledger);
    assert_eq!(
        ledger.reserve_host_mcp(&s, "r:1", b"call").unwrap(),
        HostMcpAdmission::Fresh
    );
    drop(ledger);
    let ledger = SqliteLedger::open(&path).unwrap();
    assert_eq!(
        ledger.reserve_host_mcp(&s, "r:1", b"call").unwrap(),
        HostMcpAdmission::Unknown
    );
    ledger
        .lock()
        .unwrap()
        .execute("UPDATE host_state SET ingress='closed'", [])
        .unwrap();
    assert!(ledger.complete_host_mcp(&s, "r:1", b"response").is_err());
    ledger.unknown_host_mcp(&s, "r:1").unwrap();
    let state: String = ledger
        .lock()
        .unwrap()
        .query_row("SELECT state FROM host_mcp_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "unknown");
}

#[test]
fn host_mcp_foreign_owner_epoch_policy_and_terminal_turn_fail_before_admission() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let s = seed(&ledger);
    for mutate in 0..4 {
        let mut bad = s.clone();
        match mutate {
            0 => bad.coordinator_id = "foreign".into(),
            1 => bad.host_epoch = HostEpoch(2),
            2 => bad.policy_id = "foreign".into(),
            _ => bad.run_id = "foreign".into(),
        }
        assert!(ledger.reserve_host_mcp(&bad, "r:1", b"call").is_err());
    }
    ledger
        .lock()
        .unwrap()
        .execute("UPDATE turns SET phase='completed'", [])
        .unwrap();
    assert!(ledger.reserve_host_mcp(&s, "r:1", b"call").is_err());
    let count: u64 = ledger
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM host_mcp_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn host_mcp_generation_binding_unknown_and_response_limits_are_enforced() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let s = seed(&ledger);
    ledger.reserve_host_mcp(&s, "r:1", b"call").unwrap();
    let mut foreign = s.clone();
    foreign.source_sha256 = "c".repeat(64);
    assert!(ledger.unknown_host_mcp(&foreign, "r:1").is_err());
    assert!(
        ledger
            .complete_host_mcp(&s, "r:1", &vec![0; MAX_BYTES + 1])
            .is_err()
    );
    ledger.unknown_host_mcp(&s, "r:1").unwrap();
    assert!(ledger.complete_host_mcp(&s, "r:1", b"result").is_err());
    assert_eq!(
        ledger.reserve_host_mcp(&s, "r:1", b"call").unwrap(),
        HostMcpAdmission::Unknown
    );
}
