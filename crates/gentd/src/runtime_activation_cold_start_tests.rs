use super::*;

fn stale_work(fixture: &Fixture, state: &str) -> rusqlite::Connection {
    let _ledger = fixture.ledger();
    let connection = rusqlite::Connection::open(fixture.data.join("gent.db")).unwrap();
    connection.execute_batch(
        "INSERT INTO conversations(conversation_id) VALUES ('legacy');
         INSERT INTO runs(run_id,provider,conversation_id) VALUES ('legacy-run','codex','legacy');
         INSERT INTO turns(turn_id,conversation_id,run_id,sequence,phase) VALUES ('legacy-turn','legacy','legacy-run',1,'active');
         INSERT INTO conversation_messages(message_id,turn_id,conversation_id,run_id,text,text_digest_sha256,byte_len)
          VALUES ('legacy-message','legacy-turn','legacy','legacy-run','x','digest',1);
         INSERT INTO agent_chat_prompt_dispatches(message_id,state,coordinator_id,host_epoch,created_rowid)
          VALUES ('legacy-message','claimed','dead-owner',1,1);
         INSERT INTO pending_provider_permissions(decision_id,conversation_id,run_id,binding_json,request_json)
          VALUES ('legacy-permission','legacy','legacy-run','{}','{}');"
    ).unwrap();
    connection
        .execute("UPDATE agent_chat_prompt_dispatches SET state=?1", [state])
        .unwrap();
    connection
}

fn snapshot(connection: &rusqlite::Connection) -> (u64, String, String, u64, String) {
    connection
        .query_row(
            "SELECT epoch,ingress,(SELECT state FROM agent_chat_prompt_dispatches),
          (SELECT count(*) FROM pending_provider_permissions),(SELECT phase FROM turns)
         FROM host_state WHERE singleton=1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap()
}

#[test]
fn offline_activation_preserves_open_epoch_and_stale_work_for_successor_recovery() {
    for state in ["claimed", "launching", "started"] {
        let fixture = Fixture::new("v0.2.4");
        let connection = stale_work(&fixture, state);
        let before = snapshot(&connection);
        let selected = fixture.activate().unwrap();
        assert!(selected.daemon_path.exists());
        assert_eq!(selected.origin_host_epoch, Some(1));
        assert_eq!(snapshot(&connection), before);
        assert_eq!(before, (1, "open".into(), state.into(), 1, "active".into()));
    }
}

#[test]
fn explicit_repair_still_rejects_open_or_wrong_epoch_before_store_publication() {
    let fixture = Fixture::new("v0.2.4");
    let connection = stale_work(&fixture, "started");
    let before = snapshot(&connection);
    assert!(
        activate_at_expected(
            &fixture.bootstrap,
            &fixture.root,
            &fixture.data,
            |_, _| Ok(()),
            None,
            Some(1),
            None
        )
        .is_err()
    );
    assert!(!fixture.root.exists());
    assert_eq!(snapshot(&connection), before);
    connection
        .execute("UPDATE host_state SET ingress='closed'", [])
        .unwrap();
    assert!(
        activate_at_expected(
            &fixture.bootstrap,
            &fixture.root,
            &fixture.data,
            |_, _| Ok(()),
            None,
            Some(2),
            None
        )
        .is_err()
    );
    assert!(!fixture.root.exists());
}
