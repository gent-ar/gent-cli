use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gent_ports::{AgentChatPromptDispatchLedger, AgentChatSelectionLedger, Ledger};
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{AgentChatPromptSaved, AgentChatProvider, ConversationActivityFact, HostEpoch};

use super::super::{StandalonePromptRelease, StandaloneReadiness};
use super::install_hold_tests::{held, restored, setup};
use crate::{
    ordinary_lifecycle_cadence::{
        OrdinaryLifecycleCadence, OrdinaryPromptIngress, pair_with_readiness,
    },
    ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter},
};

#[derive(Debug)]
struct Ready(bool);
impl crate::standalone_provider_readiness::StandalonePublicProviderReadiness for Ready {
    fn is_ready(&self, _: AgentChatProvider) -> Result<bool, String> {
        Ok(self.0)
    }
}

struct Host {
    ledger: SqliteLedger,
    epoch: HostEpoch,
    claims: Arc<Mutex<Vec<AgentChatPromptSaved>>>,
}
impl OrdinaryLifecycleHost for Host {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }
    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        self.ledger
            .recover_agent_chat_prompt_dispatches(self.epoch)
            .map_err(|_| ())
    }
    fn wake(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn drive(&mut self) -> Result<(), ()> {
        if let Some(saved) = self
            .ledger
            .claim_agent_chat_prompt_dispatch("fresh-owner", self.epoch, self.provider())
            .map_err(|_| ())?
        {
            self.claims.lock().unwrap().push(saved);
        }
        Ok(())
    }
    fn needs_drive(&self) -> bool {
        self.ledger
            .has_pending_agent_chat_prompt_dispatch(self.provider())
            .unwrap()
    }
    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn shutdown_complete(&self) -> bool {
        true
    }
}

type Fresh = (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<SqliteLedger>,
    OrdinaryLifecycleCadence<SqliteLedger>,
    Arc<Mutex<Vec<AgentChatPromptSaved>>>,
);

fn fresh(ledger: &SqliteLedger, epoch: HostEpoch, readiness: StandaloneReadiness) -> Fresh {
    let claims = Arc::new(Mutex::new(Vec::new()));
    let router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(ledger.clone()),
        vec![Box::new(Host {
            ledger: ledger.clone(),
            epoch,
            claims: claims.clone(),
        })],
    )
    .unwrap();
    let (control, ingress, cadence) = pair_with_readiness(Arc::new(Mutex::new(router)), readiness);
    (control, ingress, cadence, claims)
}

fn readiness(ledger: &SqliteLedger, epoch: HostEpoch, ready: bool) -> StandaloneReadiness {
    StandaloneReadiness::new_with_public_readiness(
        ledger.clone(),
        epoch,
        None,
        Arc::new(Ready(ready)),
    )
}

#[tokio::test]
async fn repaired_successor_releases_the_original_saved_prompt_once_without_resubmission() {
    let (directory, ledger, old) = setup();
    let prompt = held(&ledger, &old, "restart-ready");
    let original = ledger
        .read_awaiting_agent_chat_prompts(None, "", 128)
        .unwrap()
        .remove(0);
    ledger.quiesce_idle_owner(HostEpoch(1)).unwrap();
    drop(old);
    drop(ledger);
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let epoch = ledger.fence_and_open(HostEpoch(1)).unwrap().epoch;
    let (control, _, cadence, claims) = fresh(&ledger, epoch, readiness(&ledger, epoch, true));
    let notify = cadence.notify.clone();
    let task = tokio::spawn(cadence.run());
    tokio::time::timeout(Duration::from_secs(2), control.wait_until_ready())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while claims.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    for _ in 0..3 {
        notify.notify_one();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let delivered = claims.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].receipt, original.receipt);
    assert_eq!(delivered[0].message, original.message);
    assert_eq!(delivered[0].tool_source_ids, original.tool_source_ids);
    assert_eq!(restored(&ledger,&prompt).iter().filter(|fact| matches!(fact,
        ConversationActivityFact::PromptReleased {message_id,..} if message_id==&original.message.message_id)).count(),1);
    control.request_shutdown();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn still_unready_successor_keeps_one_hold_and_cancels_only_the_exact_original_turn() {
    let (_directory, ledger, old) = setup();
    let prompt = held(&ledger, &old, "restart-not-ready");
    ledger.quiesce_idle_owner(HostEpoch(1)).unwrap();
    drop(old);
    let epoch = ledger.fence_and_open(HostEpoch(1)).unwrap().epoch;
    let restored_readiness = readiness(&ledger, epoch, false);
    let (control, _, cadence, claims) = fresh(&ledger, epoch, restored_readiness.clone());
    let task = tokio::spawn(cadence.run());
    tokio::time::timeout(Duration::from_secs(2), control.wait_until_ready())
        .await
        .unwrap()
        .unwrap();
    assert!(claims.lock().unwrap().is_empty());
    assert_eq!(
        restored_readiness
            .waiters
            .held_for_run(&prompt.run)
            .unwrap()
            .len(),
        1
    );
    assert!(
        !restored_readiness
            .cancel_held_turn("wrong-run", &prompt.turn_id)
            .unwrap()
    );
    assert!(
        !restored_readiness
            .cancel_held_turn(&prompt.run, "wrong-turn")
            .unwrap()
    );
    assert_eq!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 128)
            .unwrap()
            .len(),
        1
    );
    assert!(
        restored_readiness
            .cancel_held_turn(&prompt.run, &prompt.turn_id)
            .unwrap()
    );
    assert!(
        !restored_readiness
            .cancel_held_turn(&prompt.run, &prompt.turn_id)
            .unwrap()
    );
    assert!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 128)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restored(&ledger, &prompt)
            .iter()
            .filter(|fact| matches!(fact, ConversationActivityFact::PromptCanceled { .. }))
            .count(),
        1
    );
    control.request_shutdown();
    task.await.unwrap().unwrap();
}

#[test]
fn fresh_readiness_cancels_durable_hold_even_before_any_waiter_reconstruction() {
    let (_directory, ledger, old) = setup();
    let prompt = held(&ledger, &old, "cancel-before-recovery");
    drop(old);
    let fresh = readiness(&ledger, HostEpoch(1), false);
    assert!(fresh.waiters.held_for_run(&prompt.run).unwrap().is_empty());
    assert!(
        fresh
            .cancel_held_turn(&prompt.run, &prompt.turn_id)
            .unwrap()
    );
    assert!(
        !fresh
            .cancel_held_turn(&prompt.run, &prompt.turn_id)
            .unwrap()
    );
}

#[test]
fn a_switch_carries_a_held_prompt_to_the_new_selection_without_starving_others() {
    let (_directory, ledger, old) = setup();
    let stale = held(&ledger, &old, "old-selection");
    let current = held(&ledger, &old, "current-selection");
    ledger
        .switch_agent_chat_selection(&gent_types::AgentChatSelectionSwitch {
            receipt_id: gent_types::ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: HostEpoch(1),
            conversation_id: gent_types::AgentChatConversationId(stale.conversation.clone()),
            parent_run_id: gent_types::AgentChatRunId(stale.run.clone()),
            run_id: gent_types::AgentChatRunId("new-selection".into()),
            selection: gent_types::AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "new-model".into(),
                effort: gent_types::AgentChatEffort::Medium,
                mode: gent_types::AgentChatMode::Agent,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        })
        .unwrap();
    drop(old);
    let fresh = readiness(&ledger, HostEpoch(1), false);
    let page = ledger
        .read_awaiting_agent_chat_prompts(None, "", 128)
        .unwrap();
    assert_eq!(page.len(), 2);
    let carried = page
        .iter()
        .find(|saved| saved.receipt.receipt_id.0 == stale.receipt)
        .unwrap();
    assert_eq!(carried.run_id.0, "new-selection");
    assert!(
        page.iter()
            .any(|saved| saved.receipt.receipt_id.0 == current.receipt)
    );
    assert!(!fresh.cancel_held_turn(&stale.run, &stale.turn_id).unwrap());
    let wrong = crate::agent_chat_api::PromptWake {
        conversation_id: gent_types::AgentChatConversationId(current.conversation.clone()),
        run_id: gent_types::AgentChatRunId("wrong-run".into()),
        receipt_id: gent_types::ReceiptId(current.receipt.clone()),
        disposition: gent_types::AgentChatPromptDisposition::Send,
    };
    assert!(fresh.release(&wrong).is_err());
    assert_eq!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 128)
            .unwrap()
            .len(),
        2
    );
}
