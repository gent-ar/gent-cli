use super::*;
use gent_types::*;

fn fixture() -> (
    PolicyRecord,
    PermissionDecisionRequest,
    PermissionDecisionResponse,
) {
    let policy = PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::AskEveryTime,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    let pending = PermissionDecisionRequest {
        binding: PermissionDecisionBinding {
            decision_id: AgentChatDecisionId("decision".into()),
            request_idempotency_key: "request".into(),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            turn_id: "turn".into(),
            policy_id: policy.policy_id.clone(),
            policy_revision: 1,
            host_epoch: HostEpoch(3),
            request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
        },
        request: PermissionRequest::new(
            "ExternalWrite".into(),
            PermissionCategory::Provider,
            None,
            None,
        ),
    };
    let response = PermissionDecisionResponse {
        binding: pending.binding.clone(),
        response: PermissionDecisionResponseKind::ApproveExactTool,
        input: None,
    };
    (policy, pending, response)
}

#[test]
fn canonical_grant_id_matches_an_independent_sha256_vector() {
    let (policy, pending, response) = fixture();
    let next = transition(&policy, &pending, &response).unwrap().unwrap();
    assert_eq!(
        next.policy_id,
        "provider-permissions-v2-542bc547a33628f5c866fdce8a982efe59831a79149e94ff8ce7c8ab95b0dbed"
    );
    assert_eq!(next.allowed_tools, ["ExternalWrite"]);
    assert_eq!(next.revision, 2);
    assert_eq!(next.mode, PermissionMode::AskEveryTime);
}

#[test]
fn a_grant_extends_the_latest_revision_saved_after_the_request_was_bound() {
    let (mut policy, pending, response) = fixture();
    policy.policy_id = "mode-changed-mid-turn".into();
    policy.revision = 2;
    policy.mode = PermissionMode::Bypass;
    let next = transition(&policy, &pending, &response).unwrap().unwrap();
    assert_eq!(next.revision, 3);
    assert_eq!(next.mode, PermissionMode::Bypass);
    assert_eq!(next.allowed_tools, ["ExternalWrite"]);
}

#[test]
fn a_changed_response_binding_cannot_publish_any_grant() {
    let (policy, pending, mut response) = fixture();
    response.binding.host_epoch = HostEpoch(4);
    assert!(transition(&policy, &pending, &response).is_err());
}

#[test]
fn existing_grants_and_once_or_deny_never_create_a_redundant_revision() {
    let (mut policy, pending, mut response) = fixture();
    for kind in [
        PermissionDecisionResponseKind::ApproveOnce,
        PermissionDecisionResponseKind::Deny,
    ] {
        response.response = kind;
        assert_eq!(transition(&policy, &pending, &response), Ok(None));
    }
    response.response = PermissionDecisionResponseKind::ApproveExactTool;
    policy.allowed_tools = vec![pending.request.tool_name.clone()];
    assert_eq!(transition(&policy, &pending, &response), Ok(None));
}
