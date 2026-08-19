use gent_core::DecisionCommandOutcome;
use gent_protocol::{
    AgentChatIntentFrame, DecisionRecoveryEvidence, DependencyAction, DependencyActionRequest,
    DependencyPlanRequest, DependencyProvider, PublicRunInterruptRequest, PublicRunOutcome,
    PublicRunResumeRequest, PublicRunStartRequest,
};
use gent_runtime::catalog::{
    CatalogError, declared_capabilities, declared_capabilities_with_agent_chat,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    CapabilitySet, Command, DecisionCommand, DecisionSettlement, DecisionSettlementPhase,
    EventPage, HostEpoch, McpPermissionStatus, ReceiptId,
};
use serde_json::json;

use crate::api::RuntimeApi;

use super::{RuntimeFacade, build_runtime};
use crate::decision_mapping::{recovery as decision_recovery, submission as decision_submission};
fn runtime() -> (tempfile::TempDir, RuntimeFacade) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &declared_capabilities(),
        super::CompatibilityAssessment::default(),
    )
    .unwrap();
    (directory, runtime)
}
#[test]
fn drifted_handlers_are_rejected_before_a_runtime_can_advertise_them() {
    let directory = tempfile::tempdir().unwrap();
    let mut observed = declared_capabilities();
    observed.0.push("future-handler".into());
    let error = build_runtime(
        directory.path(),
        &observed,
        super::CompatibilityAssessment::default(),
    )
    .unwrap_err();

    assert_eq!(
        error.downcast_ref::<CatalogError>(),
        Some(&CatalogError::UndeclaredObserved("future-handler".into()))
    );
    assert!(!directory.path().join("gent.db").exists());
}
#[test]
fn facade_exposes_only_durable_or_read_only_observer_operations() {
    let (_directory, runtime) = runtime();
    assert_eq!(runtime.capabilities().unwrap(), declared_capabilities());
    let status = runtime.status().unwrap();
    assert_eq!(status.host_epoch, HostEpoch(1));
    assert_eq!(
        status.capabilities,
        CapabilitySet(declared_capabilities().0)
    );

    let receipt = runtime
        .submit(Command {
            receipt_id: ReceiptId("receipt".into()),
            idempotency_key: "key".into(),
            host_epoch: status.host_epoch,
            kind: "test".into(),
            payload: json!({"safe": true}),
        })
        .unwrap();
    assert_eq!(receipt.idempotency_key, "key");
    assert!(matches!(
        runtime.read_event_page(0, 100).unwrap(),
        EventPage { events, next_after_cursor: None } if events.len() == 2
    ));
    assert_eq!(
        runtime.doctor().mcp.permission,
        McpPermissionStatus::HardDisabledObserver
    );
    assert!(
        runtime
            .dependency_plan(DependencyPlanRequest {
                provider: DependencyProvider::Claude,
                action: DependencyAction::Install,
            })
            .consent_required
    );
    assert_observer_operations(&runtime, status.host_epoch);
}
fn assert_observer_operations(runtime: &RuntimeFacade, epoch: HostEpoch) {
    assert_eq!(
        runtime
            .dependency_action(DependencyActionRequest {
                provider: DependencyProvider::Codex,
                action: DependencyAction::Update,
                consent_granted: false,
                receipt_id: ReceiptId("dependency".into()),
                idempotency_key: "dependency".into(),
                host_epoch: epoch,
                reviewed_plan_digest: "reviewed".into(),
            })
            .unwrap()
            .state,
        gent_protocol::DependencyActionState::ConsentRequired
    );
    let plan = runtime.dependency_plan(DependencyPlanRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Install,
    });
    assert_eq!(
        runtime
            .dependency_action(DependencyActionRequest {
                provider: DependencyProvider::Codex,
                action: DependencyAction::Install,
                consent_granted: true,
                receipt_id: ReceiptId("observer-dependency".into()),
                idempotency_key: "observer-dependency".into(),
                host_epoch: epoch,
                reviewed_plan_digest: plan.reviewed_plan_digest,
            })
            .unwrap()
            .state,
        gent_protocol::DependencyActionState::Failed
    );
    assert_eq!(
        runtime
            .start_public_run(PublicRunStartRequest {
                run_id: "run".into(),
                coordinator_id: "test".into(),
                host_epoch: epoch,
                provider: DependencyProvider::Claude,
                executable: "not-used".into(),
                version: "1".into(),
                compatibility_entry: "fixture".into(),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(
        runtime
            .resume_public_run(PublicRunResumeRequest {
                run_id: "missing-run".into(),
                coordinator_id: "test".into(),
                host_epoch: epoch,
                session_id: "ignored".into(),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(
        runtime
            .interrupt_public_run(PublicRunInterruptRequest {
                run_id: "missing-run".into(),
                coordinator_id: "test".into(),
                host_epoch: epoch,
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(
        runtime
            .conversation_status("missing")
            .unwrap()
            .conversation_id,
        "missing"
    );
    assert!(runtime.conversations().unwrap().is_empty());
}

#[test]
fn facade_decisions_are_idempotent_and_terminal() {
    let (_directory, runtime) = runtime();
    let command = DecisionCommand {
        decision_id: "decision".into(),
        idempotency_key: "key".into(),
    };
    assert!(matches!(
        runtime.submit_decision(command.clone()).unwrap(),
        gent_protocol::DecisionSubmission::Accepted(_)
    ));
    assert!(matches!(
        runtime.submit_decision(command).unwrap(),
        gent_protocol::DecisionSubmission::Duplicate(_)
    ));
    assert_eq!(
        runtime
            .apply_decision_recovery(
                "decision".into(),
                DecisionRecoveryEvidence::AcknowledgementUnprovable
            )
            .unwrap()
            .phase,
        DecisionSettlementPhase::Unprovable
    );
}

#[test]
fn approved_agent_chat_profile_persists_create_and_prompt_without_a_provider() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &declared_capabilities_with_agent_chat(true),
        super::CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create-1".into()),
            receipt_id: ReceiptId("receipt-create".into()),
            workspace_path: ".".into(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            receipt,
            ..
        },
    ] = &created[..]
    else {
        panic!("approved profile must return durable chat identities");
    };
    assert_eq!(receipt.status, gent_types::ReceiptStatus::Settled);
    let prompt = runtime
        .agent_chat_intent(AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("prompt-1".into()),
            receipt_id: ReceiptId("receipt-prompt".into()),
            conversation_id: conversation_id.clone(),
            text: "hello".into(),
        })
        .unwrap();
    assert!(matches!(
        prompt.as_slice(),
        [AgentChatIntentFrame::Accepted { receipt, delivery: gent_types::AgentChatPromptDelivery::AwaitingProvider, .. }]
            if receipt.status == gent_types::ReceiptStatus::Settled
    ));
    assert_eq!(
        runtime
            .conversation_content(&conversation_id.0, None, 10)
            .unwrap()
            .entries[0]
            .run_id,
        run_id.0
    );
}

#[test]
fn helper_mappings_preserve_all_public_outcomes() {
    let decision = DecisionSettlement {
        decision_id: "decision".into(),
        idempotency_key: "key".into(),
        phase: DecisionSettlementPhase::Pending,
    };
    assert!(matches!(
        decision_submission(DecisionCommandOutcome::Accepted(decision.clone())),
        gent_protocol::DecisionSubmission::Accepted(_)
    ));
    assert!(matches!(
        decision_submission(DecisionCommandOutcome::Duplicate(decision)),
        gent_protocol::DecisionSubmission::Duplicate(_)
    ));
    assert!(matches!(
        decision_submission(DecisionCommandOutcome::IdempotencyConflict {
            existing_decision_id: "decision".into()
        }),
        gent_protocol::DecisionSubmission::IdempotencyConflict { .. }
    ));
    assert!(matches!(
        decision_submission(DecisionCommandOutcome::DecisionIdConflict {
            existing_idempotency_key: "key".into()
        }),
        gent_protocol::DecisionSubmission::DecisionIdConflict { .. }
    ));
    assert_eq!(
        decision_recovery(DecisionRecoveryEvidence::AcknowledgementUnprovable),
        gent_core::DecisionEvidence::AcknowledgementUnprovable
    );
    assert_eq!(
        decision_recovery(DecisionRecoveryEvidence::RecoveryRequired),
        gent_core::DecisionEvidence::RecoveryRequired
    );
}
