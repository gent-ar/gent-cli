//! Integration checks for durable-chat read composition, separate from observer tests.

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AgentChatConversationFrame, AgentChatIntentFrame, AgentChatTranscriptFrame,
    ORCHESTRATION_CAPABILITY, PROVIDER_READINESS_CAPABILITY, ProviderReadinessFrame,
    ProviderReadinessReviewState, REVIEWED_PLAN_CAPABILITY,
};
use gent_runtime::catalog::{
    RuntimeCapabilityFeature, RuntimeCapabilityProfile, declared_capabilities_with_profiles,
    validate_observed_capabilities,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    ReceiptId,
};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn durable_chat_authority_advertises_and_serves_only_normalized_read_models() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]);
    let capabilities = crate::transport::observed_capabilities(&profile);
    assert_eq!(
        validate_observed_capabilities(&capabilities).unwrap(),
        declared_capabilities_with_profiles(&profile)
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_CONVERSATIONS_CAPABILITY.into())
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_TRANSCRIPT_CAPABILITY.into())
    );
    assert!(capabilities.0.contains(&ORCHESTRATION_CAPABILITY.into()));
    assert!(!capabilities.0.contains(&REVIEWED_PLAN_CAPABILITY.into()));
    assert!(
        !capabilities
            .0
            .contains(&PROVIDER_READINESS_CAPABILITY.into())
    );
    let runtime = build_runtime(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
            workspace_path: ".".into(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id, ..
        },
    ] = created.as_slice()
    else {
        panic!("durable chat profile must create one conversation");
    };
    let summary = runtime
        .agent_chat_conversation(AgentChatConversationFrame::SummaryRequest {
            conversation_id: conversation_id.0.clone(),
        })
        .unwrap();
    assert!(matches!(
        summary,
        AgentChatConversationFrame::Summary(value)
            if value.selection.provider == AgentChatProvider::Codex
    ));
    let transcript = runtime
        .agent_chat_transcript(AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.0.clone(),
            after_cursor: None,
            limit: 20,
        })
        .unwrap();
    assert!(matches!(
        transcript,
        AgentChatTranscriptFrame::Page(value)
            if value.conversation_id == conversation_id.0 && value.events.is_empty()
    ));
}

#[test]
fn explicit_readiness_profile_returns_only_daemon_derived_install_review() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ProviderReadiness,
    ]);
    let runtime = build_runtime(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("readiness-request".into()),
            receipt_id: ReceiptId("readiness-receipt".into()),
            workspace_path: ".".into(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("chat creation must return one durable run");
    };
    assert!(matches!(
        runtime
            .provider_readiness(ProviderReadinessFrame::Assess {
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
            })
            .unwrap(),
        ProviderReadinessFrame::Review { conversation_id: reply_conversation, run_id: reply_run, state: ProviderReadinessReviewState::MissingInstall, plan }
            if reply_conversation == *conversation_id && reply_run == *run_id
                && plan.provider == gent_protocol::DependencyProvider::Codex
    ));
    assert!(
        runtime
            .provider_readiness(ProviderReadinessFrame::Assess {
                conversation_id: conversation_id.clone(),
                run_id: gent_types::AgentChatRunId("stale-run".into()),
            })
            .unwrap_err()
            .contains("staleAgentChatRun")
    );
}
