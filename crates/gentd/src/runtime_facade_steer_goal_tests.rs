use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use gent_ports::{AgentChatPromptDispatchLedger, AgentChatPromptLedger, GoalLedger};
use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatReadService, GoalAuthority, GoalResult, GoalService,
    catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile},
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, GoalStatus, GoalStatusReason, HostEpoch, ReceiptId,
};

use super::*;
use crate::{
    CompatibilityAssessment,
    api::RuntimeApi,
    ordinary_lifecycle_cadence::{AsyncOrdinaryLifecycleHost, pair_with_standalone_readiness},
    ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter,
};

#[derive(Debug)]
struct Claurst(Arc<Mutex<Vec<String>>>);

#[async_trait]
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
    async fn interrupt_claurst_run(&mut self, run_id: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(run_id.into());
        Ok(())
    }
    fn shutdown_complete(&self) -> bool {
        true
    }
}

fn prompt(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    key: &str,
    disposition: AgentChatPromptDisposition,
) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(key.into()),
            receipt_id: ReceiptId(format!("receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition,
            text: key.into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    saved
}

fn running(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
) -> AgentChatPromptSaved {
    let active = prompt(
        ledger,
        conversation_id,
        "active",
        AgentChatPromptDisposition::Send,
    );
    ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Claurst)
        .unwrap()
        .unwrap();
    let id = &active.message.message_id;
    ledger
        .begin_agent_chat_prompt_launch(id, "daemon", HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(id, "daemon", HostEpoch(1))
        .unwrap();
    active
}

#[tokio::test]
async fn a_claurst_steer_interrupts_without_pausing_the_goal_while_a_user_stop_pauses_it() {
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
    let ledger = state.ledger().clone();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(AgentChatReadService::new(ledger.clone()), vec![])
            .unwrap(),
    ));
    let (control, ingress, cadence) =
        pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let interrupts = Arc::new(Mutex::new(Vec::new()));
    ingress
        .attach_async_claurst(Box::new(Claurst(Arc::clone(&interrupts))))
        .await
        .unwrap();
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
    let task = tokio::spawn(cadence.run());
    while control.phase() != crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Ready {
        tokio::task::yield_now().await;
    }
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create".into()),
            receipt_id: ReceiptId("create-receipt".into()),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: "qwen3-1-7b-q4-k-m".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            }),
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
    let goals = GoalService::new(ledger.clone(), GoalAuthority::Approved);
    let GoalResult::Goal(Some(_)) = goals
        .set(
            "goal",
            conversation_id,
            "Keep going".into(),
            None,
            HostEpoch(1),
            1_000,
        )
        .unwrap()
    else {
        panic!("goal must be set")
    };
    let active = running(&ledger, conversation_id);
    let queued = prompt(
        &ledger,
        conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );

    let steered = runtime
        .agent_chat_intent(AgentChatIntentFrame::SteerQueuedPrompt {
            request_id: AgentChatRequestId("steer".into()),
            receipt_id: ReceiptId("steer-receipt".into()),
            conversation_id: conversation_id.clone(),
            message_id: queued.message.message_id.clone(),
        })
        .unwrap();
    assert!(matches!(
        steered.as_slice(),
        [AgentChatIntentFrame::QueuedPromptSteered { .. }]
    ));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while interrupts.lock().unwrap().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        *interrupts.lock().unwrap(),
        *std::slice::from_ref(&run_id.0)
    );
    assert_eq!(
        ledger
            .current_goal(&conversation_id.0)
            .unwrap()
            .unwrap()
            .status,
        GoalStatus::Active
    );
    ledger
        .settle_agent_chat_prompt_terminal(
            &active.message.message_id,
            "daemon",
            HostEpoch(1),
            gent_types::DurableTurnPhase::Interrupted,
        )
        .unwrap();
    assert_eq!(
        terminal_fact(&ledger, &conversation_id.0, &run_id.0),
        Some((
            gent_types::TurnPhase::Interrupted,
            Some(gent_types::TurnTerminalCause::Steered)
        ))
    );

    runtime
        .agent_chat_intent(AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId("stop".into()),
            receipt_id: ReceiptId("stop-receipt".into()),
            conversation_id: conversation_id.clone(),
            run_id: AgentChatRunId(run_id.0.clone()),
        })
        .unwrap();
    let paused = ledger.current_goal(&conversation_id.0).unwrap().unwrap();
    assert_eq!(
        (paused.status, paused.reason),
        (GoalStatus::Paused, GoalStatusReason::UserStopped)
    );
    task.abort();
}

fn terminal_fact(
    ledger: &gent_store::SqliteLedger,
    conversation_id: &str,
    run_id: &str,
) -> Option<(gent_types::TurnPhase, Option<gent_types::TurnTerminalCause>)> {
    gent_ports::ConversationActivityLedger::read_conversation_activity_page(
        ledger,
        conversation_id,
        run_id,
        0,
        50,
    )
    .unwrap()
    .facts
    .into_iter()
    .find_map(|fact| match fact {
        gent_types::ConversationActivityFact::Terminal { phase, cause, .. } => Some((phase, cause)),
        _ => None,
    })
}
