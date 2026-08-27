use std::time::Duration;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
    async_claurst_interrupts: Arc<Mutex<VecDeque<String>>>,
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
    fn shutdown_complete(&self) -> bool;
}

impl<L> OrdinaryPromptIngress<L> {
    /// Acquires admission before a durable prompt transaction begins.
    pub(crate) fn acquire_prompt(
        &self,
    ) -> Result<OrdinaryPromptPermit, OrdinaryPromptAdmissionError> {
        self.control.acquire_prompt()
    }

    pub(crate) fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .respond_codex_permission(run_id, request_id, decision, answers)
            .map_err(|_| "Codex permission owner is unavailable".to_owned())
    }

    pub(crate) fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), String> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    pub(crate) fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .respond_claude_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(|_| "Claude permission owner is unavailable".to_owned())
    }

    pub(crate) fn interrupt_run(
        &self,
        provider: AgentChatProvider,
        run_id: &str,
    ) -> Result<(), String> {
        if provider == AgentChatProvider::Claurst {
            if let Some(readiness) = &self.standalone_readiness
                && readiness.cancel_claurst_provision(run_id)?
            {
                return Ok(());
            }
            if !self.async_claurst_attached.load(Ordering::Acquire) {
                return Err("Claurst interrupt owner is unavailable".into());
            }
            self.async_claurst_interrupts
                .lock()
                .map_err(|_| "Claurst interrupt queue is unavailable".to_owned())?
                .push_back(run_id.to_owned());
            if let Some(notify) = &self.async_claurst_notify {
                notify.notify_one();
            }
            return Ok(());
        }
        self.wake
            .router
            .lock()
            .map_err(|_| "standalone lifecycle router is unavailable".to_owned())?
            .interrupt_run(provider, run_id)
            .map_err(|_| "provider interrupt owner is unavailable".to_owned())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OrdinaryLifecycleCadence<L> {
    pub(super) router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    pub(super) notify: Arc<Notify>,
    pub(super) control: OrdinaryLifecycleControl,
    pub(super) async_claurst: Arc<tokio::sync::Mutex<Option<Box<dyn AsyncOrdinaryLifecycleHost>>>>,
    pub(super) async_claurst_notify: Arc<Notify>,
    pub(super) async_claurst_interrupts: Arc<Mutex<VecDeque<String>>>,
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
    let async_claurst_attached = Arc::new(AtomicBool::new(false));
    let control = OrdinaryLifecycleControl::new();
    (
        control.clone(),
        OrdinaryPromptIngress {
            control: control.clone(),
            wake: OrdinaryPromptWake {
                router: Arc::clone(&router),
                notify: Arc::clone(&notify),
            },
            standalone_readiness: None,
            async_claurst_notify: Some(Arc::clone(&async_claurst_notify)),
            async_claurst: Arc::clone(&async_claurst),
            async_claurst_interrupts: Arc::clone(&async_claurst_interrupts),
            async_claurst_attached,
        },
        OrdinaryLifecycleCadence {
            router,
            notify,
            control,
            async_claurst,
            async_claurst_notify,
            async_claurst_interrupts,
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
    let (control, mut ingress, cadence) = pair(router);
    ingress.standalone_readiness =
        Some(Arc::new(StandaloneReadiness::new(ledger, host_epoch, None)));
    (control, ingress, cadence)
}

#[must_use]
pub(crate) fn pair_with_standalone_models(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    ledger: SqliteLedger,
    host_epoch: HostEpoch,
    models: crate::standalone_authority_composition::StandaloneClaurstModels,
) -> (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<SqliteLedger>,
    OrdinaryLifecycleCadence<SqliteLedger>,
) {
    let (control, mut ingress, cadence) = pair(router);
    ingress.standalone_readiness = Some(Arc::new(StandaloneReadiness::new(
        ledger,
        host_epoch,
        Some(models),
    )));
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
            if readiness.release(&prompt)? == StandalonePromptReleaseOutcome::Claurst {
                self.async_claurst_notify
                    .as_ref()
                    .expect("ordinary cadence always retains its Claurst wake")
                    .notify_one();
                return Ok(());
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
mod wake;

#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_shutdown_tests.rs"]
mod ordinary_lifecycle_cadence_shutdown_tests;
#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_tests.rs"]
mod ordinary_lifecycle_cadence_tests;
