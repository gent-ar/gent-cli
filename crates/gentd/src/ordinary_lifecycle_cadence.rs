use std::time::Duration;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use async_trait::async_trait;
use gent_ports::AgentChatReadLedger;
use gent_store::SqliteLedger;
use gent_types::{AgentChatProvider, HostEpoch};
use tokio::sync::Notify;

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_control::{
    OrdinaryLifecycleControl, OrdinaryPromptAdmissionError, OrdinaryPromptPermit,
};
use crate::ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter;
use standalone::{StandalonePromptRelease, StandalonePromptReleaseOutcome, StandaloneReadiness};

pub(super) const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub(crate) struct OrdinaryPromptWake<L> {
    control: OrdinaryLifecycleControl,
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    notify: Arc<Notify>,
}

#[derive(Clone, Debug)]
pub(crate) struct OrdinaryPromptIngress<L> {
    control: OrdinaryLifecycleControl,
    wake: OrdinaryPromptWake<L>,
    standalone_readiness: Option<Arc<dyn StandalonePromptRelease>>,
    async_claurst_notify: Option<Arc<Notify>>,
    async_claurst: Arc<tokio::sync::Mutex<Option<Box<dyn AsyncOrdinaryLifecycleHost>>>>,
    async_claurst_interrupts: Arc<Mutex<VecDeque<PendingClaurstInterrupt>>>,
    async_claurst_warm: Arc<Mutex<Option<crate::provider_warm_pool::WarmTarget>>>,
    async_claurst_attached: Arc<AtomicBool>,
}

#[async_trait]
pub(crate) trait AsyncOrdinaryLifecycleHost: Send + std::fmt::Debug {
    async fn activate_recovery(&mut self) -> Result<(), String>;
    async fn drive_once(&mut self) -> Result<bool, String>;
    async fn begin_shutdown_after_recovery(&mut self) -> Result<(), String>;
    async fn respond_claurst_permission(
        &mut self,
        _response: gent_types::PermissionDecisionResponse,
    ) -> Result<(), String> {
        Err("Claurst permission owner is unavailable".into())
    }
    async fn respond_claurst_permission_with_receipt(
        &mut self,
        _response: gent_types::PermissionDecisionResponse,
        _receipt_id: gent_types::ReceiptId,
    ) -> Result<gent_types::Receipt, String> {
        Err("Claurst permission receipt authority is unavailable".into())
    }
    async fn interrupt_claurst_run(&mut self, _run_id: &str) -> Result<(), String> {
        Err("Claurst interrupt owner is unavailable".into())
    }
    async fn interrupt_claurst_turn(
        &mut self,
        _run_id: &str,
        _turn_id: &str,
    ) -> Result<(), String> {
        Err("Claurst turn-bound interrupt owner is unavailable".into())
    }
    async fn warm_claurst(&mut self, _conversation_id: &str, _run_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn shutdown_complete(&self) -> bool;
    fn is_idle(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InterruptGoalStop {
    pub(crate) goals: gent_runtime::GoalService<SqliteLedger>,
    pub(crate) conversation_id: gent_types::AgentChatConversationId,
    pub(crate) host_epoch: HostEpoch,
}
impl InterruptGoalStop {
    fn apply(self) {
        if let Err(error) = self.goals.stop(
            &self.conversation_id,
            self.host_epoch,
            crate::startup::unix_seconds(),
        ) {
            eprintln!("goal stop after validated interruption was not recorded: {error}");
        }
    }
}

#[derive(Debug)]
pub(super) struct PendingClaurstInterrupt {
    run_id: String,
    expected_turn_id: Option<String>,
    goal_stop: Option<InterruptGoalStop>,
}

#[derive(Clone, Debug)]
pub(crate) struct OrdinaryLifecycleCadence<L> {
    standalone_recovery: Option<Arc<dyn held_recovery::StandaloneHeldRecovery>>,
    pub(super) router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    pub(super) notify: Arc<Notify>,
    pub(super) control: OrdinaryLifecycleControl,
    pub(super) async_claurst: Arc<tokio::sync::Mutex<Option<Box<dyn AsyncOrdinaryLifecycleHost>>>>,
    pub(super) async_claurst_notify: Arc<Notify>,
    pub(super) async_claurst_interrupts: Arc<Mutex<VecDeque<PendingClaurstInterrupt>>>,
    pub(super) async_claurst_warm: Arc<Mutex<Option<crate::provider_warm_pool::WarmTarget>>>,
}

#[must_use]
pub(crate) fn pair<L>(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
) -> (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<L>,
    OrdinaryLifecycleCadence<L>,
) {
    let notify = Arc::new(Notify::new());
    let async_claurst_notify = Arc::new(Notify::new());
    let async_claurst = Arc::new(tokio::sync::Mutex::new(None));
    let async_claurst_interrupts = Arc::new(Mutex::new(VecDeque::new()));
    let async_claurst_warm = Arc::new(Mutex::new(None));
    let async_claurst_attached = Arc::new(AtomicBool::new(false));
    let control = OrdinaryLifecycleControl::new();
    (
        control.clone(),
        OrdinaryPromptIngress {
            control: control.clone(),
            wake: OrdinaryPromptWake {
                control: control.clone(),
                router: Arc::clone(&router),
                notify: Arc::clone(&notify),
            },
            standalone_readiness: None,
            async_claurst_notify: Some(Arc::clone(&async_claurst_notify)),
            async_claurst: Arc::clone(&async_claurst),
            async_claurst_interrupts: Arc::clone(&async_claurst_interrupts),
            async_claurst_warm: Arc::clone(&async_claurst_warm),
            async_claurst_attached,
        },
        OrdinaryLifecycleCadence {
            standalone_recovery: None,
            router,
            notify,
            control,
            async_claurst,
            async_claurst_notify,
            async_claurst_interrupts,
            async_claurst_warm,
        },
    )
}

#[must_use]
pub(crate) fn pair_with_standalone_readiness(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    ledger: SqliteLedger,
    host_epoch: HostEpoch,
) -> (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<SqliteLedger>,
    OrdinaryLifecycleCadence<SqliteLedger>,
) {
    pair_with_readiness(router, StandaloneReadiness::new(ledger, host_epoch, None))
}

#[must_use]
pub(crate) fn pair_with_standalone_models(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    ledger: SqliteLedger,
    host_epoch: HostEpoch,
    models: crate::standalone_authority_composition::StandaloneClaurstModels,
    public_readiness: crate::standalone_provider_readiness::StandalonePublicProviderReadinessPort,
) -> (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<SqliteLedger>,
    OrdinaryLifecycleCadence<SqliteLedger>,
) {
    pair_with_readiness(
        router,
        StandaloneReadiness::new_with_public_readiness(
            ledger,
            host_epoch,
            Some(models),
            public_readiness,
        ),
    )
}

fn pair_with_readiness(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    readiness: StandaloneReadiness,
) -> (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<SqliteLedger>,
    OrdinaryLifecycleCadence<SqliteLedger>,
) {
    let (control, mut ingress, mut cadence) = pair(router);
    ingress.standalone_readiness = Some(Arc::new(readiness));
    cadence.standalone_recovery = Some(Arc::new(ingress.clone()));
    (control, ingress, cadence)
}

impl<L: AgentChatReadLedger + 'static> PromptCommitWake for OrdinaryPromptIngress<L> {
    type Error = String;

    fn handles_awaiting_readiness(&self) -> bool {
        true
    }

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        if let Some(readiness) = &self.standalone_readiness {
            if readiness.provider(&prompt)? == AgentChatProvider::Claurst {
                readiness.provision_claurst(
                    prompt,
                    Arc::clone(
                        self.async_claurst_notify
                            .as_ref()
                            .expect("ordinary cadence always retains its Claurst wake"),
                    ),
                )?;
                return Ok(());
            }
            match readiness.release(&prompt)? {
                StandalonePromptReleaseOutcome::Claurst => {
                    self.async_claurst_notify
                        .as_ref()
                        .expect("ordinary cadence always retains its Claurst wake")
                        .notify_one();
                    return Ok(());
                }
                StandalonePromptReleaseOutcome::Held => return Ok(()),
                StandalonePromptReleaseOutcome::Routed => {}
            }
            self.wake.schedule(prompt, Arc::clone(readiness));
            return Ok(());
        }
        self.wake.wake_after_prompt_commit(prompt)
    }
}

#[path = "ordinary_lifecycle_cadence_claurst.rs"]
mod claurst;
#[path = "ordinary_lifecycle_cadence_run.rs"]
mod run;

#[path = "ordinary_lifecycle_cadence_standalone.rs"]
mod standalone;

#[path = "ordinary_lifecycle_cadence_wake.rs"]
pub(crate) mod wake;

#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_shutdown_tests.rs"]
mod ordinary_lifecycle_cadence_shutdown_tests;
#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_tests.rs"]
mod ordinary_lifecycle_cadence_tests;

#[path = "ordinary_lifecycle_cadence_ingress.rs"]
mod ingress;
#[path = "ordinary_lifecycle_owner_control.rs"]
mod owner_control;

#[path = "ordinary_lifecycle_held_recovery.rs"]
mod held_recovery;
