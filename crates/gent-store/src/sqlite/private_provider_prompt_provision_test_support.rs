use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger, PrivateProviderPromptProvisionLedger,
    ReceiptClaim,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, Command, Event, HostEpoch, ProviderInstallProvenance,
    ProviderPromptProvisionBinding, ProviderPromptProvisionCommandBinding,
    ProviderPromptProvisionPackageBinding, ProvisionedProviderInstallation,
    ProvisionedProviderLock, Receipt, ReceiptId, RunVersionLock, WorkspaceRecord,
};
use serde_json::json;

use super::SqliteLedger;

pub(super) fn seeded() -> (SqliteLedger, gent_types::AgentChatPromptSaved) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationCreate {
        receipt_id: ReceiptId("conversation-receipt".into()),
        idempotency_key: "conversation-key".into(),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Agent,
        },
    };
    ledger
        .create_agent_chat_conversation_in_workspace(
            &conversation,
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "install then send".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    (ledger, saved)
}

pub(super) fn provision(
    ledger: &SqliteLedger,
    saved: &gent_types::AgentChatPromptSaved,
) -> (ProviderPromptProvisionCommandBinding, Command, Receipt) {
    let binding = ProviderPromptProvisionCommandBinding {
        prompt: ProviderPromptProvisionBinding {
            prompt_receipt_id: saved.receipt.receipt_id.clone(),
            conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
            run_id: saved.run_id.clone(),
            provider: "codex".into(),
            action: "install".into(),
            consent_granted: true,
            reviewed_plan_digest: "a".repeat(64),
        },
        expected_reviewed_plan_digest: "a".repeat(64),
        release_artifact_digest_sha256: "d".repeat(64),
        package: ProviderPromptProvisionPackageBinding {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
        },
    };
    let command = Command {
        receipt_id: ReceiptId("provision-receipt".into()),
        idempotency_key: "provision-key".into(),
        host_epoch: HostEpoch(1),
        kind: "providerPromptProvision".into(),
        payload: serde_json::to_value(&binding).unwrap(),
    };
    let receipt = match ledger
        .claim_command(
            &command,
            &Event {
                cursor: 0,
                event_id: "provision-accepted".into(),
                receipt_id: command.receipt_id.clone(),
                host_epoch: command.host_epoch,
                kind: "accepted".into(),
                payload: json!({}),
            },
        )
        .unwrap()
    {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("new provision command must be accepted"),
    };
    ledger
        .reserve_verified_provider_prompt_provision(&command, &binding)
        .unwrap();
    (binding, command, receipt)
}

pub(super) fn settle(
    ledger: &SqliteLedger,
    binding: &ProviderPromptProvisionCommandBinding,
    command: &Command,
    receipt: &Receipt,
) -> Result<Receipt, gent_ports::LedgerError> {
    ledger.settle_verified_provider_prompt_provision(
        command,
        receipt,
        &installation(command, binding),
        &terminal(receipt),
        binding,
    )
}

pub(super) fn terminal(receipt: &Receipt) -> Event {
    Event {
        cursor: 0,
        event_id: "provision-installed".into(),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "privatePromptProvisionInstalled".into(),
        payload: json!({}),
    }
}

pub(super) fn installation(
    command: &Command,
    binding: &ProviderPromptProvisionCommandBinding,
) -> ProvisionedProviderInstallation {
    ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock {
            run_lock: RunVersionLock {
                provider: "codex".into(),
                canonical_path: "/private/codex".into(),
                file_identity: "identity".into(),
                digest_sha256: "a".repeat(64),
                version: "1.0.0".into(),
                compatibility_entry: "codex-1".into(),
            },
        },
        provenance: ProviderInstallProvenance {
            package_name: "@openai/codex".into(),
            package_version: "1.0.0".into(),
            package_integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
            node_runtime_digest_sha256: "c".repeat(64),
            release_artifact_digest_sha256: binding.release_artifact_digest_sha256.clone(),
            receipt_fingerprint_sha256: command.receipt_fingerprint_sha256(),
        },
    }
}

pub(super) fn dispatch_state(ledger: &SqliteLedger, message_id: &str) -> String {
    ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM agent_chat_prompt_dispatches WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
}

pub(super) fn receipt_status(ledger: &SqliteLedger, key: &str) -> String {
    ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT status FROM receipts WHERE idempotency_key = ?1",
            [key],
            |row| row.get(0),
        )
        .unwrap()
}
