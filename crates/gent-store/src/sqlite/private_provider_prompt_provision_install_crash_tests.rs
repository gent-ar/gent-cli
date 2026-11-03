use super::*;

fn interrupted() -> (SqliteLedger, String, Command) {
    let (ledger, message, command, _, _) = reserved_prompt();
    (ledger, message, command)
}

#[test]
fn successor_fences_interrupted_install_without_forging_old_owner_completion() {
    let (ledger, message, command) = interrupted();
    let before = installer_snapshot(&ledger);
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(1))
        .unwrap();
    assert_eq!(dispatch_state(&ledger, &message), "provisioning");
    assert_eq!(
        installer_snapshot(&ledger),
        before,
        "same-epoch live install must remain owned"
    );
    restart(&ledger);
    assert_eq!(dispatch_state(&ledger, &message), "awaiting_readiness");
    let connection = ledger.lock().unwrap();
    let (epoch, payload): (u64, String) = connection
        .query_row(
            "SELECT host_epoch,payload FROM events WHERE kind='privatePromptProvisionRecovered'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(epoch, 2);
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(
        value["originalCommand"],
        serde_json::to_value(&command).unwrap()
    );
    assert_eq!(
        value["recoveryHostEpoch"],
        serde_json::to_value(HostEpoch(2)).unwrap()
    );
    assert_eq!(
        value["originalCommandFingerprint"],
        crate::sqlite::queries::command_fingerprint(&command).unwrap()
    );
    drop(connection);
    let recovered = installer_snapshot(&ledger);
    assert!(recovered.0.starts_with("unprovable:"));
    assert_eq!(recovered.1.len(), before.1.len() + 1);
    assert_eq!(recovered.1[0], before.1[0]);
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert_eq!(installer_snapshot(&ledger), recovered);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("successor", HostEpoch(2), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[test]
fn interrupted_install_recovery_rejects_missing_provenance_or_other_effect_ownership() {
    for mutation in [
        "UPDATE receipts SET payload_digest='' WHERE idempotency_key='provision-key'",
        "UPDATE receipts SET status='settled' WHERE idempotency_key='provision-key'",
        "UPDATE receipts SET kind='' WHERE idempotency_key='provision-key'",
        "UPDATE events SET payload='{}' WHERE event_id='accepted'",
        "UPDATE events SET host_epoch=99 WHERE event_id='accepted'",
        "UPDATE agent_chat_prompt_dispatches SET coordinator_id='agent-owner',host_epoch=1",
        "UPDATE agent_chat_run_selections SET provider='claude'",
        "UPDATE turns SET phase='cancelled'",
        "INSERT INTO events(event_id,receipt_id,host_epoch,kind,payload) SELECT 'terminal',receipt_id,host_epoch,'privatePromptProvisionInstalled',payload FROM events WHERE event_id='accepted'",
        "INSERT INTO events(event_id,receipt_id,host_epoch,kind,payload) SELECT 'duplicate',receipt_id,host_epoch,kind,payload FROM events WHERE event_id='accepted'",
        "INSERT INTO agent_chat_steer_targets(message_id,receipt_id,run_id,turn_id,state) SELECT message_id,'prompt-receipt',run_id,turn_id,'rejected' FROM agent_chat_prompt_receipts",
    ] {
        let (ledger, message, _) = interrupted();
        ledger.lock().unwrap().execute(mutation, []).unwrap();
        restart(&ledger);
        assert_eq!(
            dispatch_state(&ledger, &message),
            "provisioning",
            "{mutation}"
        );
        assert!(
            ledger
                .read_awaiting_agent_chat_prompts(None, "", 128)
                .unwrap()
                .is_empty()
        );
    }
}
