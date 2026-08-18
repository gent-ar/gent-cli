use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, HostEpoch,
    PermissionDecisionBinding, PermissionDecisionRequest, PermissionDecisionResponse,
    PermissionDecisionResponseKind, PermissionMode, PermissionRequest, PermissionRequestDigest,
    PolicyRecord, PolicyScope, SandboxEnforcement,
};

use crate::{
    PermissionControlContext, PermissionControlEffect, PermissionControlEvent,
    PermissionControlRejection, PermissionControlResolution, PermissionControlState,
    reduce_permission_control,
};

fn context(mode: PermissionMode) -> PermissionControlContext {
    PermissionControlContext {
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
        turn_id: "turn-1".into(),
        host_epoch: HostEpoch(4),
        policy: PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "workspace-1".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 2,
            mode,
            allowed_tools: vec![],
            allowed_categories: vec![],
        },
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
        sandbox: SandboxEnforcement::Unavailable,
    }
}

fn request() -> PermissionDecisionRequest {
    PermissionDecisionRequest {
        binding: PermissionDecisionBinding {
            decision_id: AgentChatDecisionId("decision-1".into()),
            request_idempotency_key: "request-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            policy_id: "policy-1".into(),
            policy_revision: 2,
            host_epoch: HostEpoch(4),
            request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
        },
        request: PermissionRequest {
            tool_name: "workspace:edit".into(),
            category: gent_types::PermissionCategory::Edit,
        },
    }
}

#[test]
fn response_must_match_the_exact_pending_run_and_digest() {
    let context = context(PermissionMode::Default);
    let requested = reduce_permission_control(
        PermissionControlState::default(),
        &context,
        PermissionControlEvent::Request(request()),
    );
    assert!(matches!(requested.1, PermissionControlEffect::AskUser(_)));
    let mut response = PermissionDecisionResponse {
        binding: request().binding,
        response: PermissionDecisionResponseKind::ApproveOnce,
    };
    response.binding.run_id = AgentChatRunId("old-run".into());
    let rejected = reduce_permission_control(
        requested.0,
        &context,
        PermissionControlEvent::Respond(response),
    );
    assert_eq!(
        rejected.1,
        PermissionControlEffect::Rejected(PermissionControlRejection::StaleRun)
    );
    assert!(rejected.0.pending().is_some());
}

#[test]
fn approval_effects_are_closed_and_derived_from_the_pending_request() {
    let context = context(PermissionMode::Default);
    let pending = reduce_permission_control(
        PermissionControlState::default(),
        &context,
        PermissionControlEvent::Request(request()),
    );
    let response = PermissionDecisionResponse {
        binding: request().binding,
        response: PermissionDecisionResponseKind::ApproveCategory,
    };
    let resolved = reduce_permission_control(
        pending.0,
        &context,
        PermissionControlEvent::Respond(response),
    );
    assert_eq!(
        resolved.1,
        PermissionControlEffect::Resolved(PermissionControlResolution::ApproveCategory {
            category: gent_types::PermissionCategory::Edit,
        })
    );
    assert!(resolved.0.pending().is_none());
}

#[test]
fn plan_mode_fails_closed_before_a_user_response_can_expand_it() {
    let result = reduce_permission_control(
        PermissionControlState::default(),
        &context(PermissionMode::Plan),
        PermissionControlEvent::Request(request()),
    );
    assert!(matches!(
        result.1,
        PermissionControlEffect::Resolved(PermissionControlResolution::DeniedByPolicy(_))
    ));
    assert!(result.0.pending().is_none());
}

#[test]
fn policy_revision_and_digest_are_fenced_before_evaluation() {
    let mut bad = request();
    bad.binding.policy_revision = 3;
    let policy_rejected = reduce_permission_control(
        PermissionControlState::default(),
        &context(PermissionMode::Default),
        PermissionControlEvent::Request(bad),
    );
    assert_eq!(
        policy_rejected.1,
        PermissionControlEffect::Rejected(PermissionControlRejection::PolicyMismatch)
    );
    let mut bad = request();
    bad.binding.request_digest_sha256 = PermissionRequestDigest("b".repeat(64));
    let digest_rejected = reduce_permission_control(
        PermissionControlState::default(),
        &context(PermissionMode::Default),
        PermissionControlEvent::Request(bad),
    );
    assert_eq!(
        digest_rejected.1,
        PermissionControlEffect::Rejected(PermissionControlRejection::RequestDigestMismatch)
    );
}

#[test]
fn malformed_digest_is_rejected_before_a_pending_decision_is_created() {
    let mut bad = request();
    bad.binding.request_digest_sha256 = PermissionRequestDigest("not-a-digest".into());
    let result = reduce_permission_control(
        PermissionControlState::default(),
        &context(PermissionMode::Default),
        PermissionControlEvent::Request(bad),
    );
    assert_eq!(
        result.1,
        PermissionControlEffect::Rejected(PermissionControlRejection::InvalidRequestDigest)
    );
    assert!(result.0.pending().is_none());
}
