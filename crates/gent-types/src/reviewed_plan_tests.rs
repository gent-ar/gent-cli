use serde_json::json;

use super::*;
use crate::{AgentChatEffort, AgentChatMode, AgentChatProvider, ReceiptId, ReceiptStatus};

fn artifact() -> PlanArtifact {
    PlanArtifact {
        plan_id: ReviewedPlanId("plan-1".into()),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        source_run_id: AgentChatRunId("run-1".into()),
        source_turn_id: "turn-1".into(),
        revision: PlanRevision(1),
        content_digest_sha256: "a".repeat(64),
        status: PlanStatus::ReadyForReview,
        actions: vec![PlanAction {
            action_id: "action-1".into(),
            kind: PlanActionKind::Edit,
            summary: "Update a config file".into(),
        }],
        risks: vec![],
        diffs: vec![PlanDiff {
            path: "Cargo.toml".into(),
            kind: PlanDiffKind::Modify,
            summary: "Add a dependency".into(),
        }],
        permission_preview: vec![PlanPermissionPreview {
            category: PermissionCategory::Edit,
            summary: "Modify one file".into(),
        }],
    }
}

#[test]
fn artifact_is_closed_and_contains_no_raw_provider_fields() {
    let value = serde_json::to_value(artifact()).unwrap();
    assert_eq!(value["contentDigestSha256"], "a".repeat(64));
    assert!(
        serde_json::from_value::<PlanArtifact>(json!({
        "planId": "p", "conversationId": "c", "sourceRunId": "r", "sourceTurnId": "t", "revision": 1,
            "contentDigestSha256": "a".repeat(64), "status": "draft", "actions": [],
            "risks": [], "diffs": [], "permissionPreview": [], "providerSessionId": "never"
        }))
        .is_err()
    );
}

#[test]
fn artifact_rejects_unbounded_metadata_and_invalid_digest() {
    let mut value = artifact();
    value.actions[0].summary = "x".repeat(MAX_SUMMARY_BYTES + 1);
    assert_eq!(
        value.validate(),
        Err(ReviewedPlanContractError::InvalidMetadata)
    );
    value.actions[0].summary = "valid".into();
    value.content_digest_sha256 = "UPPER".into();
    assert_eq!(
        value.validate(),
        Err(ReviewedPlanContractError::InvalidRevisionOrDigest)
    );
}

#[test]
fn approval_keeps_context_policy_and_frozen_boundary() {
    let request = StartImplementationRequest {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "key-1".into(),
        host_epoch: HostEpoch(4),
        policy_workspace_id: "workspace-1".into(),
        policy_revision: 2,
        conversation_id: AgentChatConversationId("conversation-1".into()),
        plan_id: ReviewedPlanId("plan-1".into()),
        plan_revision: PlanRevision(3),
        plan_content_digest_sha256: "b".repeat(64),
        parent_run_id: AgentChatRunId("parent-1".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Plan,
        },
        context_policy: ContextPolicy::Clear,
    };
    assert!(request.validate().is_ok());
    assert_eq!(
        serde_json::to_value(request).unwrap()["contextPolicy"],
        "clear"
    );
    let _ = StartImplementationResult {
        receipt: Receipt {
            receipt_id: ReceiptId("receipt-1".into()),
            idempotency_key: "key-1".into(),
            status: ReceiptStatus::Accepted,
            host_epoch: HostEpoch(4),
        },
        conversation_id: AgentChatConversationId("conversation-1".into()),
        plan_id: ReviewedPlanId("plan-1".into()),
        plan_revision: PlanRevision(3),
        parent_run_id: AgentChatRunId("parent-1".into()),
        implementation_run_id: AgentChatRunId("child-1".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Plan,
        },
        context_policy: ContextPolicy::Clear,
        context_through_ordinal: 0,
    };
}
