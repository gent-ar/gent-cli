use std::sync::{Arc, Mutex};

use gent_ports::{AgentChatProjectionLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger};
use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatReadService,
    catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile},
};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    HostEpoch, ReceiptId,
};

use super::*;
use crate::{
    CompatibilityAssessment,
    agent_chat_intent_error::AgentChatIntentError,
    api::RuntimeApi,
    ordinary_lifecycle_cadence::{AsyncOrdinaryLifecycleHost, pair_with_standalone_readiness},
    ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter,
};

#[path = "runtime_facade_conversation_links_refusal_tests.rs"]
mod refusal_tests;
#[path = "runtime_facade_conversation_links_tests.rs"]
mod tests;

#[derive(Debug)]
struct Claurst;

#[async_trait::async_trait]
impl AsyncOrdinaryLifecycleHost for Claurst {
    async fn activate_recovery(&mut self) -> Result<(), String> {
        Ok(())
    }
    async fn drive_once(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    async fn begin_shutdown_after_recovery(&mut self) -> Result<(), String> {
        Ok(())
    }
    async fn interrupt_claurst_run(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn shutdown_complete(&self) -> bool {
        true
    }
}

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

pub(super) async fn facade(directory: &std::path::Path) -> RuntimeFacade {
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::TurnFollow,
        RuntimeCapabilityFeature::LocalModels,
    ]);
    let state =
        DaemonCompositionState::open(directory, &profile, CompatibilityAssessment::default())
            .unwrap();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(state.ledger().clone()),
            vec![],
        )
        .unwrap(),
    ));
    let (control, ingress, cadence) =
        pair_with_standalone_readiness(router, state.ledger().clone(), HostEpoch(1));
    ingress
        .attach_async_claurst(Box::new(Claurst))
        .await
        .unwrap();
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        ingress,
        crate::local_model_jobs::tests::models(directory),
        None,
        None,
        None,
        0,
        Vec::new(),
        None,
    )
    .unwrap();
    tokio::spawn(cadence.run());
    while control.phase() != crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Ready {
        tokio::task::yield_now().await;
    }
    runtime
}

pub(super) fn root(
    runtime: &RuntimeFacade,
    key: &str,
    workspace_path: &str,
) -> AgentChatConversationId {
    root_with_selection(runtime, key, workspace_path, selection())
}

pub(super) fn root_with_selection(
    runtime: &RuntimeFacade,
    key: &str,
    workspace_path: &str,
    selection: AgentChatSelection,
) -> AgentChatConversationId {
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(format!("create-{key}")),
            receipt_id: ReceiptId(format!("create-receipt-{key}")),
            workspace_path: workspace_path.into(),
            selection: Some(selection),
        })
        .unwrap();
    match created.into_iter().next() {
        Some(AgentChatIntentFrame::Created {
            conversation_id, ..
        }) => conversation_id,
        _ => panic!("a root conversation must be created"),
    }
}

fn create_linked(
    runtime: &RuntimeFacade,
    key: &str,
    parent: &AgentChatConversationId,
    label: &str,
) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
    runtime.agent_chat_intent(AgentChatIntentFrame::CreateLinkedConversation {
        request_id: AgentChatRequestId(format!("linked-{key}")),
        receipt_id: ReceiptId(format!("linked-receipt-{key}")),
        parent_conversation_id: parent.clone(),
        workspace_path: None,
        selection: None,
        label: label.into(),
        prompt: None,
        origin_tool_use_id: Some(format!("tool-{key}")),
    })
}

pub(super) fn child(
    runtime: &RuntimeFacade,
    key: &str,
    parent: &AgentChatConversationId,
    label: &str,
) -> AgentChatConversationId {
    match create_linked(runtime, key, parent, label)
        .unwrap()
        .into_iter()
        .next()
    {
        Some(AgentChatIntentFrame::LinkedConversationCreated {
            conversation_id, ..
        }) => conversation_id,
        _ => panic!("a linked conversation must be created"),
    }
}

pub(super) fn facts(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
) -> Vec<serde_json::Value> {
    runtime
        .conversation_links()
        .unwrap()
        .agent_chat_projection_page(conversation, 0, 64)
        .unwrap()
        .events
        .into_iter()
        .map(|event| event.payload)
        .filter(|payload: &serde_json::Value| payload.get("activity").is_some())
        .map(|payload| payload["activity"].clone())
        .collect()
}

pub(super) fn running(runtime: &RuntimeFacade, conversation: &AgentChatConversationId, key: &str) {
    let ledger = runtime.conversation_links().unwrap().clone();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("run-{key}")),
            receipt_id: ReceiptId(format!("run-receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "work".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Claurst)
        .unwrap()
        .unwrap();
    let id = &saved.message.message_id;
    ledger
        .begin_agent_chat_prompt_launch(id, "daemon", HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(id, "daemon", HostEpoch(1))
        .unwrap();
}
