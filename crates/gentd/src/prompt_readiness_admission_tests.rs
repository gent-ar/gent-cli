use std::cell::Cell;

use gent_drivers::lock::capture;
use gent_ports::{
    AgentChatPromptDispatchLedger, Ledger, LedgerError, ProvisionedProviderLockReader,
};
use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationService, AgentChatPromptAuthority,
    AgentChatPromptService, AgentChatReadService, AgentChatSelectionSwitchAuthority,
    AgentChatSelectionSwitchService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    ProviderInstallProvenance, ProvisionedProviderInstallation, ProvisionedProviderLock, ReceiptId,
};

use crate::agent_chat_api::{PromptCommitWake, PromptWake, exchange, exchange_with_wake};
use crate::private_provider_readiness::PrivateProviderReadinessService;

use super::ProviderReadyPromptAdmission;

#[derive(Clone)]
struct Locks(Option<ProvisionedProviderInstallation>);

impl ProvisionedProviderLockReader for Locks {
    fn find_provisioned_provider_installation(
        &self,
        _: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        Ok(self.0.clone())
    }
}

struct Wake {
    calls: Cell<u8>,
}

impl PromptCommitWake for Wake {
    type Error = std::convert::Infallible;

    fn wake_after_prompt_commit(&mut self, _: PromptWake) -> Result<(), Self::Error> {
        self.calls.set(self.calls.get().saturating_add(1));
        Ok(())
    }
}

#[test]
fn ready_daemon_fact_atomically_releases_then_notifies_the_lifecycle() {
    let (ledger, conversations, prompts, switches) = services();
    let conversation_id = create(&conversations, &prompts, &switches);
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    std::fs::write(&executable, "locked provider").unwrap();
    let lock = capture("codex", &executable, "1.0", "entry").unwrap();
    let mut wake = ProviderReadyPromptAdmission::new(
        AgentChatReadService::new(ledger.clone()),
        PrivateProviderReadinessService::new(Locks(Some(installation(lock)))),
        ledger.clone(),
        gent_types::HostEpoch(1),
        Wake {
            calls: Cell::new(0),
        },
    );
    exchange_with_wake(
        &conversations,
        &prompts,
        &switches,
        gent_types::HostEpoch(1),
        send(conversation_id),
        &mut wake,
    )
    .unwrap();
    assert_eq!(wake.next.calls.get(), 1);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                "daemon",
                gent_types::HostEpoch(1),
                AgentChatProvider::Codex,
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(
        ledger
            .read_event_page(0, 10)
            .unwrap()
            .events
            .iter()
            .filter(|event| event.kind == "agentChatProviderReady")
            .count(),
        1
    );
}

#[test]
fn missing_install_keeps_the_prompt_held_and_never_wakes() {
    let (ledger, conversations, prompts, switches) = services();
    let conversation_id = create(&conversations, &prompts, &switches);
    let mut wake = ProviderReadyPromptAdmission::new(
        AgentChatReadService::new(ledger.clone()),
        PrivateProviderReadinessService::new(Locks(None)),
        ledger.clone(),
        gent_types::HostEpoch(1),
        Wake {
            calls: Cell::new(0),
        },
    );
    exchange_with_wake(
        &conversations,
        &prompts,
        &switches,
        gent_types::HostEpoch(1),
        send(conversation_id),
        &mut wake,
    )
    .unwrap();
    assert_eq!(wake.next.calls.get(), 0);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                "daemon",
                gent_types::HostEpoch(1),
                AgentChatProvider::Codex,
            )
            .unwrap()
            .is_none()
    );
}

fn services() -> (
    SqliteLedger,
    AgentChatConversationService<SqliteLedger>,
    AgentChatPromptService<SqliteLedger>,
    AgentChatSelectionSwitchService<SqliteLedger>,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    (
        ledger.clone(),
        AgentChatConversationService::new(ledger.clone(), AgentChatConversationAuthority::Approved),
        AgentChatPromptService::new(ledger.clone(), AgentChatPromptAuthority::Approved),
        AgentChatSelectionSwitchService::new(ledger, AgentChatSelectionSwitchAuthority::Approved),
    )
}

fn create(
    conversations: &AgentChatConversationService<SqliteLedger>,
    prompts: &AgentChatPromptService<SqliteLedger>,
    switches: &AgentChatSelectionSwitchService<SqliteLedger>,
) -> gent_types::AgentChatConversationId {
    let replies = exchange(
        conversations,
        prompts,
        switches,
        gent_types::HostEpoch(1),
        AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create-request".into()),
            receipt_id: ReceiptId("create-receipt".into()),
            workspace_path: ".".into(),
            selection: selection(),
        },
    )
    .unwrap();
    match &replies[0] {
        AgentChatIntentFrame::Created {
            conversation_id, ..
        } => conversation_id.clone(),
        _ => panic!("expected one created conversation"),
    }
}

fn send(conversation_id: gent_types::AgentChatConversationId) -> AgentChatIntentFrame {
    AgentChatIntentFrame::SendPrompt {
        request_id: AgentChatRequestId("prompt-request".into()),
        receipt_id: ReceiptId("prompt-receipt".into()),
        conversation_id,
        text: "continue".into(),
    }
}

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn installation(lock: gent_types::RunVersionLock) -> ProvisionedProviderInstallation {
    ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock { run_lock: lock },
        provenance: ProviderInstallProvenance {
            package_name: "@openai/codex".into(),
            package_version: "1.0".into(),
            package_integrity: "integrity".into(),
            package_policy_digest_sha256: "a".repeat(64),
            node_runtime_digest_sha256: "b".repeat(64),
            release_artifact_digest_sha256: "c".repeat(64),
            receipt_fingerprint_sha256: "d".repeat(64),
        },
    }
}
