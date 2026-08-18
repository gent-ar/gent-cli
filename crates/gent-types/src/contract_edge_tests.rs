use std::path::PathBuf;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, ConversationContentCursor, CrossReviewRequest, GoalBinding,
    GoalContractError, GoalProjection, GoalRecord, GoalStatus, GoalTransition, HarnessProfileRef,
    HostEpoch, OnboardingState, OrchestrationContractError, PlanRevision, PlanStatus,
    ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthContractError, ProviderAuthMethod,
    ProviderAuthMethodSelection, ProviderAuthProvider, ReviewedPlanId, SandboxBackendId,
    SandboxLaunchContractError, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits,
    TaskNodeSpec, TaskRole, TurnTerminal, WorktreePolicy,
};

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn binding() -> GoalBinding {
    GoalBinding {
        goal_id: "goal-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
    }
}

#[test]
fn agent_chat_selection_rejects_a_nul_model_identifier() {
    let mut value = selection();
    value.model = "gpt\0-5.6".into();
    assert!(value.validate().is_err());
}

#[test]
fn conversation_content_cursor_rejects_zero_and_malformed_ordinals() {
    assert!(
        ConversationContentCursor::new("conversation-1", 0)
            .ordinal_for("conversation-1")
            .is_err()
    );
    let malformed = URL_SAFE_NO_PAD
        .encode("v1\0conversation-1\0not-an-ordinal")
        .parse::<ConversationContentCursor>()
        .unwrap();
    assert!(malformed.ordinal_for("conversation-1").is_err());
}

#[test]
fn goal_contract_rejects_invalid_records_and_transitions() {
    let record = GoalRecord {
        schema_version: 1,
        binding: binding(),
        revision: 0,
        status: GoalStatus::Active,
        summary: "finish the task".into(),
    };
    assert_eq!(record.validate(), Err(GoalContractError::InvalidMetadata));
    let transition = GoalTransition {
        binding: GoalBinding {
            goal_id: "\n".into(),
            ..binding()
        },
        expected_revision: 1,
        host_epoch: HostEpoch(1),
        next_status: GoalStatus::Completed,
    };
    assert_eq!(
        transition.validate(),
        Err(GoalContractError::InvalidMetadata)
    );
    let abandoned = GoalRecord {
        revision: 1,
        status: GoalStatus::Abandoned,
        ..record
    };
    assert_eq!(
        GoalProjection::from_active(&abandoned),
        Err(GoalContractError::InactiveGoal)
    );
    assert!(GoalStatus::Failed.is_terminal());
}

#[test]
fn onboarding_state_serializes_only_the_provider_neutral_projection() {
    let state = OnboardingState::from_doctor(&crate::DoctorReport::empty());
    let value = serde_json::to_value(state).unwrap();
    assert_eq!(value["branches"].as_array().unwrap().len(), 3);
    assert_eq!(value["branches"][0]["provider"], "gent");
}

#[test]
fn orchestration_cross_review_rejects_an_invalid_reviewer_role() {
    let reviewer = TaskNodeSpec {
        node_id: "reviewer-1".into(),
        role: TaskRole::Custom {
            role_id: "\0".into(),
        },
        profile: HarnessProfileRef {
            profile_id: "claude-reviewer".into(),
            revision: 1,
            provider: AgentChatProvider::Claude,
        },
        selection: AgentChatSelection {
            provider: AgentChatProvider::Claude,
            ..selection()
        },
        input_artifact_digests: vec![],
        depends_on: vec![],
        worktree: WorktreePolicy::Isolated,
        retry_budget: 0,
    };
    let request = CrossReviewRequest {
        graph_id: "graph-1".into(),
        expected_graph_revision: 1,
        expected_parent_run_id: AgentChatRunId("run-1".into()),
        host_epoch: HostEpoch(1),
        goal_revision: 1,
        policy_revision: 1,
        idempotency_key: "receipt-1".into(),
        candidate: crate::ReviewCandidate {
            node_id: "candidate-1".into(),
            node_revision: 1,
            artifact_digest_sha256: "a".repeat(64),
            base_revision_digest_sha256: "b".repeat(64),
        },
        reviewer,
    };
    assert_eq!(
        request.validate(),
        Err(OrchestrationContractError::InvalidMetadata)
    );
}

#[test]
fn provider_auth_contract_rejects_duplicate_methods_and_invalid_answers() {
    let challenge = ProviderAuthChallenge {
        challenge_id: "challenge-1".into(),
        provider: ProviderAuthProvider::Claude,
        binary_lock: ProviderAuthBinaryLock {
            canonical_executable_id: "claude:locked".into(),
            digest_sha256: "a".repeat(64),
            version: "1.0".into(),
        },
        methods: vec![
            ProviderAuthMethod::DeviceCode,
            ProviderAuthMethod::DeviceCode,
        ],
        expires_at_unix_seconds: 1,
    };
    assert_eq!(
        challenge.validate(),
        Err(ProviderAuthContractError::DuplicateMethod)
    );
    let answer = ProviderAuthMethodSelection {
        challenge_id: String::new(),
        method: ProviderAuthMethod::ApiKey,
    };
    assert_eq!(
        answer.validate(),
        Err(ProviderAuthContractError::InvalidIdentifier)
    );
}

#[test]
fn reviewed_plan_identity_and_status_remain_provider_neutral() {
    let id = serde_json::to_string(&ReviewedPlanId("plan-1".into())).unwrap();
    assert_eq!(id, "\"plan-1\"");
    assert_eq!(
        serde_json::from_str::<PlanStatus>("\"readyForReview\"").unwrap(),
        PlanStatus::ReadyForReview
    );
    assert_eq!(PlanRevision(2).0, 2);
}

#[test]
fn sandbox_profile_rejects_missing_limits_and_invalid_backend_names() {
    assert_eq!(
        SandboxBackendId::new("MacOS helper".into()),
        Err(SandboxLaunchContractError::InvalidBackend)
    );
    let result = SandboxLaunchProfile::new(
        &PathBuf::from("/workspace"),
        &[PathBuf::from("/workspace")],
        &[],
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 0,
            max_memory_bytes: 1,
            max_cpu_time_ms: 1,
        },
    );
    assert_eq!(
        result,
        Err(SandboxLaunchContractError::MissingResourceLimit)
    );
}

#[test]
fn turn_terminal_rejects_whitespace_only_public_identifiers() {
    let terminal = TurnTerminal {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: "  ".into(),
        phase: crate::DurableTurnPhase::Interrupted,
        cursor: 1,
    };
    assert!(!terminal.is_valid());
}
