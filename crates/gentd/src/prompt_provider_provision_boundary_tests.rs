use std::sync::{Arc, Mutex};

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError,
    ProvisionedProviderLockLedger,
};
use gent_protocol::{
    DependencyPlanRequest, DependencyProvider, PromptProviderProvisionFrame,
    PromptProviderProvisionState,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ProviderInstallProvenance,
    ProvisionedProviderInstallation, ProvisionedProviderLock, ReceiptId, RunVersionLock,
    WorkspaceRecord,
};

use super::{
    PrivateProvisionError, PrivateProvisionRequest, PrivateProvisionResult,
    PromptProviderProvisionBoundary, PromptProviderProvisionEffect,
};
use crate::{authority_clock::AuthorityClock, dependency_catalog::DependencyCatalog};

#[derive(Clone, Copy)]
enum Outcome {
    Installed,
    Ambiguous,
}

#[derive(Clone)]
struct Effect {
    outcome: Outcome,
    calls: Arc<Mutex<Vec<PrivateProvisionRequest>>>,
}

impl PromptProviderProvisionEffect for Effect {
    fn provision_prompt(
        &self,
        request: &PrivateProvisionRequest,
        _: &gent_types::Command,
        _: &gent_types::ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        self.calls.lock().unwrap().push(request.clone());
        Ok(match self.outcome {
            Outcome::Installed => PrivateProvisionResult::Installed(Box::new(installation())),
            Outcome::Ambiguous => PrivateProvisionResult::Ambiguous,
        })
    }
}

#[derive(Clone)]
struct Policy;

impl PackageInstallPolicy for Policy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(ApprovedPackageInstall {
            provider: provider.into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
        })
    }
}

#[derive(Clone, Copy)]
struct Clock;

impl AuthorityClock for Clock {
    fn now_unix_seconds(&self) -> u64 {
        1
    }
}

#[test]
fn completed_install_derives_the_package_and_releases_only_the_held_prompt() {
    let (ledger, saved) = seeded();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let boundary = boundary(ledger.clone(), Outcome::Installed, Arc::clone(&calls));
    let request = confirm(&saved, true, plan_digest());
    let reply = boundary.confirm(request.clone()).unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::Completed,
            ..
        }
    ));
    let retry = boundary.confirm(request).unwrap();
    assert!(matches!(
        retry,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::Completed,
            ..
        }
    ));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider, DependencyProvider::Codex);
    assert_eq!(calls[0].reviewed_plan_digest, plan_digest());
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_some()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("test", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_some()
    );
}

#[test]
fn consent_refusal_is_terminal_without_an_effect_or_prompt_release() {
    let (ledger, saved) = seeded();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let boundary = boundary(ledger.clone(), Outcome::Installed, Arc::clone(&calls));
    let reply = boundary
        .confirm(confirm(&saved, false, plan_digest()))
        .unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::ConsentRequired,
            ..
        }
    ));
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("test", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[test]
fn mismatched_review_never_calls_npm_and_keeps_the_prompt_retryable() {
    let (ledger, saved) = seeded();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let boundary = boundary(ledger.clone(), Outcome::Installed, Arc::clone(&calls));
    let reply = boundary
        .confirm(confirm(&saved, true, "a".repeat(64)))
        .unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::PlanMismatch,
            ..
        }
    ));
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("test", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[test]
fn ambiguous_effect_is_unprovable_and_an_exact_retry_never_replays_it() {
    let (ledger, saved) = seeded();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let boundary = boundary(ledger, Outcome::Ambiguous, Arc::clone(&calls));
    let request = confirm(&saved, true, plan_digest());
    for _ in 0..2 {
        let reply = boundary.confirm(request.clone()).unwrap();
        assert!(matches!(
            reply,
            PromptProviderProvisionFrame::Result {
                state: PromptProviderProvisionState::Unprovable,
                ..
            }
        ));
    }
    assert_eq!(calls.lock().unwrap().len(), 1);
}

fn boundary(
    ledger: SqliteLedger,
    outcome: Outcome,
    calls: Arc<Mutex<Vec<PrivateProvisionRequest>>>,
) -> PromptProviderProvisionBoundary<SqliteLedger, Policy, Effect, Clock> {
    PromptProviderProvisionBoundary::new(
        ledger,
        DependencyCatalog::default(),
        Policy,
        Effect { outcome, calls },
        Clock,
    )
}

fn plan_digest() -> String {
    DependencyCatalog::default()
        .plan(DependencyPlanRequest {
            provider: DependencyProvider::Codex,
            action: gent_protocol::DependencyAction::Install,
        })
        .reviewed_plan_digest
}

fn confirm(
    saved: &gent_types::AgentChatPromptSaved,
    consent_granted: bool,
    reviewed_plan_digest: String,
) -> PromptProviderProvisionFrame {
    PromptProviderProvisionFrame::Confirm {
        receipt_id: ReceiptId("provision-receipt".into()),
        idempotency_key: "provision-key".into(),
        host_epoch: HostEpoch(1),
        prompt_receipt_id: saved.receipt.receipt_id.clone(),
        conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
        run_id: saved.run_id.clone(),
        consent_granted,
        reviewed_plan_digest,
    }
}

fn seeded() -> (SqliteLedger, gent_types::AgentChatPromptSaved) {
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
        })
        .unwrap();
    (ledger, saved)
}

fn installation() -> ProvisionedProviderInstallation {
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
        },
    }
}
