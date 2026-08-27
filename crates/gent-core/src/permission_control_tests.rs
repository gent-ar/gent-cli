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
            input: None,
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
        input: None,
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
        input: None,
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

#[test]
fn pending_requests_are_idempotent_and_responses_cannot_target_another_decision() {
    let context = context(PermissionMode::Default);
    let pending = reduce_permission_control(
        PermissionControlState::default(),
        &context,
        PermissionControlEvent::Request(request()),
    );
    let duplicate = reduce_permission_control(
        pending.0,
        &context,
        PermissionControlEvent::Request(request()),
    );
    assert!(matches!(duplicate.1, PermissionControlEffect::AskUser(_)));
    let mut response = PermissionDecisionResponse {
        binding: request().binding,
        response: PermissionDecisionResponseKind::Deny,
        input: None,
    };
    response.binding.decision_id = AgentChatDecisionId("decision-2".into());
    let rejected = reduce_permission_control(
        duplicate.0,
        &context,
        PermissionControlEvent::Respond(response),
    );
    assert_eq!(
        rejected.1,
        PermissionControlEffect::Rejected(PermissionControlRejection::PendingDecisionMismatch)
    );
}

#[test]
fn one_pending_request_blocks_competitors_and_response_types_are_closed() {
    let context = context(PermissionMode::Default);
    let pending = reduce_permission_control(
        PermissionControlState::default(),
        &context,
        PermissionControlEvent::Request(request()),
    );
    let mut another = request();
    another.binding.request_idempotency_key = "request-2".into();
    assert_eq!(
        reduce_permission_control(
            pending.0.clone(),
            &context,
            PermissionControlEvent::Request(another)
        )
        .1,
        PermissionControlEffect::Rejected(PermissionControlRejection::AnotherDecisionPending)
    );
    let response = PermissionDecisionResponse {
        binding: request().binding,
        response: PermissionDecisionResponseKind::ApproveExactTool,
        input: None,
    };
    assert_eq!(
        reduce_permission_control(
            pending.0,
            &context,
            PermissionControlEvent::Respond(response)
        )
        .1,
        PermissionControlEffect::Resolved(PermissionControlResolution::ApproveExactTool {
            tool_name: "workspace:edit".into(),
        })
    );
}

#[test]
fn policy_allowance_and_missing_sandbox_skip_the_pending_state() {
    let mut allowed = context(PermissionMode::Default);
    allowed.policy.allowed_tools.push("workspace:edit".into());
    assert!(matches!(
        reduce_permission_control(
            PermissionControlState::default(),
            &allowed,
            PermissionControlEvent::Request(request())
        )
        .1,
        PermissionControlEffect::Resolved(PermissionControlResolution::AllowedByPolicy(_))
    ));
    assert_eq!(
        reduce_permission_control(
            PermissionControlState::default(),
            &context(PermissionMode::Autonomous),
            PermissionControlEvent::Request(request())
        )
        .1,
        PermissionControlEffect::Rejected(PermissionControlRejection::SandboxRequired)
    );
}

#[test]
fn a_response_without_a_request_and_stale_bindings_fail_closed() {
    let context = context(PermissionMode::Default);
    let response = PermissionDecisionResponse {
        binding: request().binding,
        response: PermissionDecisionResponseKind::Deny,
        input: None,
    };
    assert_eq!(
        reduce_permission_control(
            PermissionControlState::default(),
            &context,
            PermissionControlEvent::Respond(response)
        )
        .1,
        PermissionControlEffect::Rejected(PermissionControlRejection::NoPendingDecision)
    );
    let mut stale = request();
    stale.binding.host_epoch = HostEpoch(3);
    assert_eq!(
        reduce_permission_control(
            PermissionControlState::default(),
            &context,
            PermissionControlEvent::Request(stale)
        )
        .1,
        PermissionControlEffect::Rejected(PermissionControlRejection::StaleHostEpoch)
    );
}
