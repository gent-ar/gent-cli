use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, PackageInstallPolicy,
    PackageInstallPolicyError,
};
use gent_protocol::{
    DependencyAction, DependencyPlanRequest, DependencyProvider, PromptProviderProvisionFrame,
    PromptProviderProvisionState, ProviderInstallReview,
};
use gent_types::*;

use crate::{
    authority_clock::AuthorityClock,
    dependency_catalog::DependencyCatalog,
    private_provider_provisioning::{
        PrivateProvisionError, PrivateProvisionRequest, PrivateProvisionResult,
    },
    prompt_provider_provision_boundary::{
        PromptProviderProvisionBoundary, PromptProviderProvisionEffect,
    },
};

#[derive(Clone)]
struct Clock;
impl AuthorityClock for Clock {
    fn now_unix_seconds(&self) -> u64 {
        1
    }
}
#[derive(Clone)]
struct Policy;
impl PackageInstallPolicy for Policy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<gent_ports::ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(gent_ports::ApprovedPackageInstall {
            provider: provider.into(),
            package_name: "@openai/codex".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "a".repeat(64),
        })
    }
}
#[derive(Clone)]
struct NoEffect;
impl PromptProviderProvisionEffect for NoEffect {
    fn provision_prompt(
        &self,
        _: &PrivateProvisionRequest,
        _: &Command,
        _: &ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        panic!("old consent must never reach installation")
    }
}

#[test]
fn clean_copy_rule_changes_review_and_old_consent_is_rejected_before_effect() {
    let catalog = DependencyCatalog::default();
    let review =
        super::install_review(&catalog, &Policy, &Clock, DependencyProvider::Codex).unwrap();
    assert!(
        review
            .instruction
            .contains("Gent will install and verify a clean copy for this request.")
    );
    assert!(
        review
            .instruction
            .contains("Previous incomplete installations are left unchanged.")
    );
    assert!(review.is_valid());
    let old = ProviderInstallReview::reviewed(
        DependencyProvider::Codex,
        DependencyAction::Install,
        "Use OpenAI's supported Codex installer after reviewing its terms.",
        true,
        review.package.clone(),
    );
    assert_ne!(review.reviewed_plan_digest, old.reviewed_plan_digest);
    assert_eq!(
        catalog
            .plan(DependencyPlanRequest {
                provider: DependencyProvider::Codex,
                action: DependencyAction::Install
            })
            .instruction,
        review.instruction
    );

    let ledger = gent_store::SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation".into());
    let run = AgentChatRunId("run".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation".into()),
                idempotency_key: "conversation".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: run.clone(),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "model".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "original prompt".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let boundary = PromptProviderProvisionBoundary::new(
        ledger,
        catalog,
        Policy,
        NoEffect,
        Clock,
        "b".repeat(64),
    );
    let result = boundary
        .confirm(PromptProviderProvisionFrame::Confirm {
            receipt_id: ReceiptId("install".into()),
            idempotency_key: "install".into(),
            host_epoch: HostEpoch(1),
            prompt_receipt_id: prompt.receipt.receipt_id,
            conversation_id: conversation,
            run_id: run,
            consent_granted: true,
            reviewed_plan_digest: old.reviewed_plan_digest,
        })
        .unwrap();
    assert!(matches!(
        result,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::PlanMismatch,
            ..
        }
    ));
}
