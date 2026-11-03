use super::*;
use gent_ports::{AgentChatPromptDispatchLedger, PromptAdmission};
use rusqlite::params;

fn legacy() -> (
    SqliteLedger,
    String,
    Command,
    Receipt,
    ProviderPromptProvisionCommandBinding,
) {
    let (ledger, message, command, receipt, binding) = reserved_prompt();
    ledger
        .settle_unprovable_provider_prompt_provision(
            &command,
            &receipt,
            &Event {
                cursor: 0,
                event_id: "legacy-terminal".into(),
                receipt_id: receipt.receipt_id.clone(),
                host_epoch: receipt.host_epoch,
                kind: "privatePromptProvisionUnprovable".into(),
                payload: command.payload.clone(),
            },
            &binding,
        )
        .unwrap();
    ledger
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state='unprovable' WHERE message_id=?1",
            [&message],
        )
        .unwrap();
    (ledger, message, command, receipt, binding)
}

fn restart(ledger: &SqliteLedger) {
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
}

fn installer_snapshot(ledger: &SqliteLedger) -> (String, Vec<String>) {
    let connection = ledger.lock().unwrap();
    let receipt = connection.query_row("SELECT status||':'||payload_digest FROM receipts WHERE idempotency_key='provision-key'", [], |row| row.get(0)).unwrap();
    let events = connection.prepare("SELECT event_id||':'||kind||':'||payload FROM events WHERE receipt_id='provision-receipt' ORDER BY cursor").unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    (receipt, events)
}

#[test]
fn restart_restores_only_original_undelivered_prompt_and_never_replays_install() {
    let (ledger, message, _, receipt, binding) = legacy();
    let before = installer_snapshot(&ledger);
    restart(&ledger);
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert_eq!(dispatch_state(&ledger, &message), "awaiting_readiness");
    assert_eq!(
        ledger
            .agent_chat_prompt_admission(&binding.prompt.prompt_receipt_id)
            .unwrap(),
        PromptAdmission::Held
    );
    let prompts = ledger
        .read_awaiting_agent_chat_prompts(Some("run"), "", 128)
        .unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].message.message_id, message);
    assert_eq!(
        prompts[0].receipt.receipt_id,
        binding.prompt.prompt_receipt_id
    );
    assert_eq!(installer_snapshot(&ledger), before);
    assert_eq!(
        ledger
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM receipts WHERE receipt_id=?1",
                [receipt.receipt_id.0],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "unprovable"
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("successor", HostEpoch(2), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[test]
fn legacy_recovery_requires_exact_install_and_never_started_agent_provenance() {
    for mutation in [
        "UPDATE receipts SET payload_digest='' WHERE idempotency_key='provision-key'",
        "UPDATE receipts SET payload_digest='wrong' WHERE idempotency_key='provision-key'",
        "UPDATE receipts SET kind='' WHERE idempotency_key='provision-key'",
        "UPDATE events SET payload='{}' WHERE event_id='legacy-terminal'",
        "UPDATE events SET host_epoch=99 WHERE event_id='legacy-terminal'",
        "UPDATE events SET kind='privatePromptProvisionInstalled' WHERE event_id='legacy-terminal'",
        "DELETE FROM events WHERE event_id='accepted'",
        "UPDATE agent_chat_prompt_dispatches SET coordinator_id='provider-owner',host_epoch=1",
        "UPDATE agent_chat_prompt_receipts SET disposition='steer'",
        "INSERT INTO agent_chat_steer_targets(message_id,receipt_id,run_id,turn_id,state) SELECT message_id,'prompt-receipt',run_id,turn_id,'rejected' FROM agent_chat_prompt_receipts",
        "UPDATE agent_chat_run_selections SET provider='claude'",
        "UPDATE turns SET phase='cancelled'",
        "UPDATE turns SET phase='failed'",
        "INSERT INTO events(event_id,receipt_id,host_epoch,kind,payload) SELECT 'duplicate-terminal',receipt_id,host_epoch,kind,payload FROM events WHERE event_id='legacy-terminal'",
        "UPDATE receipts SET status='accepted' WHERE idempotency_key='provision-key'",
    ] {
        let (ledger, message, _, _, _) = legacy();
        ledger.lock().unwrap().execute(mutation, []).unwrap();
        restart(&ledger);
        assert_eq!(
            dispatch_state(&ledger, &message),
            "unprovable",
            "{mutation}"
        );
        assert!(
            ledger
                .read_awaiting_agent_chat_prompts(None, "", 128)
                .unwrap()
                .is_empty(),
            "{mutation}"
        );
    }
}

#[test]
fn restored_original_prompt_can_be_cancelled_without_touching_uncertain_installer() {
    use gent_types::{PromptAdmissionExit, ProviderPromptReadinessFailureBinding};
    let (ledger, message, _, _, install) = legacy();
    let before = installer_snapshot(&ledger);
    restart(&ledger);
    let binding = ProviderPromptReadinessFailureBinding {
        prompt_receipt_id: install.prompt.prompt_receipt_id,
        conversation_id: install.prompt.conversation_id,
        run_id: install.prompt.run_id,
        provider: AgentChatProvider::Codex,
        exit: PromptAdmissionExit::Cancelled,
        reason: "cancelled by the user".into(),
    };
    let command = Command {
        receipt_id: ReceiptId("cancel-original".into()),
        idempotency_key: "cancel-original".into(),
        host_epoch: HostEpoch(2),
        kind: "agentChatProviderReadinessFailure".into(),
        payload: serde_json::to_value(&binding).unwrap(),
    };
    let event = Event {
        cursor: 0,
        event_id: "cancel-original".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "agentChatProviderReadinessFailed".into(),
        payload: command.payload.clone(),
    };
    ledger
        .fail_verified_agent_chat_prompt_after_readiness(&command, &event, &binding)
        .unwrap();
    assert_eq!(dispatch_state(&ledger, &message), "settled");
    assert_eq!(installer_snapshot(&ledger), before);
    assert!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 128)
            .unwrap()
            .is_empty()
    );
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert_eq!(dispatch_state(&ledger, &message), "settled");
}

#[test]
fn newer_in_flight_install_cannot_be_reopened_by_an_older_terminal_event() {
    let (ledger, message, command, _, _) = legacy();
    let connection = ledger.lock().unwrap();
    connection.execute("INSERT INTO receipts(idempotency_key,receipt_id,status,host_epoch,kind,payload_digest) VALUES ('new-install','new-install','accepted',1,'providerPromptProvision','new')", []).unwrap();
    connection.execute("INSERT INTO events(event_id,receipt_id,host_epoch,kind,payload) VALUES ('new-install','new-install',1,'privatePromptProvisionAccepted',?1)", params![command.payload.to_string()]).unwrap();
    drop(connection);
    restart(&ledger);
    assert_eq!(dispatch_state(&ledger, &message), "unprovable");
}

#[path = "private_provider_prompt_provision_install_crash_tests.rs"]
mod crash_tests;
