use std::sync::{Arc, Mutex};

use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatReadService,
    catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile},
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    ContextPolicy, ReceiptId,
};

use super::*;
use crate::{
    CompatibilityAssessment, api::RuntimeApi,
    ordinary_lifecycle_cadence::pair_with_standalone_readiness,
    ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter,
};

#[test]
fn standalone_facade_accepts_curated_claurst_create_and_agent_switch() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::TurnFollow,
        RuntimeCapabilityFeature::LocalModels,
    ]);
    let state = DaemonCompositionState::open(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(state.ledger().clone()),
            vec![],
        )
        .unwrap(),
    ));
    let (_, ingress, _) =
        pair_with_standalone_readiness(router, state.ledger().clone(), gent_types::HostEpoch(1));
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        ingress,
        crate::local_model_jobs::tests::models(directory.path()),
        None,
        None,
        None,
        0,
        Vec::new(),
        None,
    )
    .unwrap();
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Ask,
    };
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create".into()),
            receipt_id: ReceiptId("create-receipt".into()),
            workspace_path: ".".into(),
            selection: Some(selection),
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
        panic!("standalone Claurst creation must succeed")
    };
    let switched = runtime
        .agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId("switch".into()),
            receipt_id: ReceiptId("switch-receipt".into()),
            conversation_id: conversation_id.clone(),
            parent_run_id: run_id.clone(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: "qwen3-1-7b-q4-k-m".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            },
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
    assert!(matches!(
        switched.as_slice(),
        [AgentChatIntentFrame::Switched { .. }]
    ));
}

#[test]
fn standalone_facade_rejects_unlisted_claurst_models() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::LocalModels,
    ]);
    let state = DaemonCompositionState::open(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(state.ledger().clone()),
            vec![],
        )
        .unwrap(),
    ));
    let (_, ingress, _) =
        pair_with_standalone_readiness(router, state.ledger().clone(), gent_types::HostEpoch(1));
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        ingress,
        crate::local_model_jobs::tests::models(directory.path()),
        None,
        None,
        None,
        0,
        Vec::new(),
        None,
    )
    .unwrap();
    let result = runtime.agent_chat_intent(AgentChatIntentFrame::CreateConversation {
        request_id: AgentChatRequestId("create-unknown".into()),
        receipt_id: ReceiptId("create-unknown-receipt".into()),
        workspace_path: ".".into(),
        selection: Some(AgentChatSelection {
            provider: AgentChatProvider::Claurst,
            model: "not-in-curated-catalogue".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Ask,
        }),
    });
    assert!(result.is_err());
}

#[test]
fn reading_an_unknown_conversation_is_a_typed_not_found_rejection() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::LocalModels,
    ]);
    let state = DaemonCompositionState::open(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(state.ledger().clone()),
            vec![],
        )
        .unwrap(),
    ));
    let (_, ingress, _) =
        pair_with_standalone_readiness(router, state.ledger().clone(), gent_types::HostEpoch(1));
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        ingress,
        crate::local_model_jobs::tests::models(directory.path()),
        None,
        None,
        None,
        0,
        Vec::new(),
        None,
    )
    .unwrap();

    let summary = runtime
        .agent_chat_conversation(gent_protocol::AgentChatConversationFrame::SummaryRequest {
            conversation_id: "conversation-missing".into(),
        })
        .unwrap_err();
    let transcript = runtime
        .agent_chat_transcript(gent_protocol::AgentChatTranscriptFrame::PageRequest {
            conversation_id: "conversation-missing".into(),
            after_cursor: None,
            limit: 10,
        })
        .unwrap_err();

    assert_eq!(summary.code, "conversationNotFound");
    assert_eq!(transcript.code, "conversationNotFound");
}
