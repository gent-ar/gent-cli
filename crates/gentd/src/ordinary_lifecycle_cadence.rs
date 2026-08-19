//! Demand-driven cadence for the private ordinary provider lifecycle.
//!
//! It owns no durable state. Prompts first commit through the ledger, then its wake adapter arms
//! the selected host and retains one notification. Polling exists only while a host reports
//! active work; settled sessions wait without a timer until the next durable prompt.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gent_ports::AgentChatReadLedger;
use tokio::sync::Notify;

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_control::{
    OrdinaryLifecycleControl, OrdinaryPromptAdmissionError, OrdinaryPromptPermit,
};
use crate::ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter;

const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Prompt-commit adapter that arms a durable-selection host before notifying the cadence.
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryPromptWake<L> {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    notify: Arc<Notify>,
}

/// Sealed post-commit wake and pre-transaction admission pair for one ordinary authority.
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryPromptIngress<L> {
    control: OrdinaryLifecycleControl,
    wake: OrdinaryPromptWake<L>,
}

impl<L> OrdinaryPromptIngress<L> {
    /// Acquires admission before a durable prompt transaction begins.
    pub(crate) fn acquire_prompt(
        &self,
    ) -> Result<OrdinaryPromptPermit, OrdinaryPromptAdmissionError> {
        self.control.acquire_prompt()
    }
}

/// One daemon-owned cadence paired with [`OrdinaryPromptWake`].
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryLifecycleCadence<L> {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    notify: Arc<Notify>,
    control: OrdinaryLifecycleControl,
}

/// Creates the only prompt wake and cadence pair for one ordinary lifecycle router.
#[must_use]
pub(crate) fn pair<L>(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
) -> (
    OrdinaryLifecycleControl,
    OrdinaryPromptIngress<L>,
    OrdinaryLifecycleCadence<L>,
) {
    let notify = Arc::new(Notify::new());
    let control = OrdinaryLifecycleControl::new();
    (
        control.clone(),
        OrdinaryPromptIngress {
            control: control.clone(),
            wake: OrdinaryPromptWake {
                router: Arc::clone(&router),
                notify: Arc::clone(&notify),
            },
        },
        OrdinaryLifecycleCadence {
            router,
            notify,
            control,
        },
    )
}

impl<L: AgentChatReadLedger> PromptCommitWake for OrdinaryPromptWake<L> {
    type Error = String;

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        self.router
            .lock()
            .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
            .wake_after_prompt_commit(prompt)
            .map_err(|_| "ordinary lifecycle wake was rejected".to_owned())?;
        self.notify.notify_one();
        Ok(())
    }
}

impl<L: AgentChatReadLedger> PromptCommitWake for OrdinaryPromptIngress<L> {
    type Error = String;

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        self.wake.wake_after_prompt_commit(prompt)
    }
}

impl<L: Send + 'static> OrdinaryLifecycleCadence<L> {
    /// Runs durable recovery once, then waits without polling until a committed prompt wakes it.
    pub(crate) async fn run(self) -> Result<(), String> {
        if self.control.phase()
            == crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Draining
        {
            return Ok(());
        }
        self.activate_recovery().await?;
        self.drive_until_idle().await?;
        if self.control.phase()
            == crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Draining
        {
            return self.drain_shutdown().await;
        }
        self.control.open_after_recovery();
        loop {
            tokio::select! {
                () = self.control.shutdown_requested() => return self.drain_shutdown().await,
                () = self.notify.notified() => self.drive_until_idle().await?,
            }
        }
    }

    async fn activate_recovery(&self) -> Result<(), String> {
        let router = Arc::clone(&self.router);
        tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                .activate_recovery()
                .map_err(|_| "ordinary lifecycle recovery was rejected".to_owned())
        })
        .await
        .map_err(|_| "ordinary lifecycle recovery task failed".to_owned())?
    }

    async fn drive_until_idle(&self) -> Result<(), String> {
        loop {
            let router = Arc::clone(&self.router);
            let active = tokio::task::spawn_blocking(move || {
                router
                    .lock()
                    .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                    .drive_once()
                    .map_err(|_| "ordinary lifecycle drive was rejected".to_owned())
            })
            .await
            .map_err(|_| "ordinary lifecycle drive task failed".to_owned())??;
            if !active {
                return Ok(());
            }
            tokio::time::sleep(ACTIVE_POLL_INTERVAL).await;
        }
    }

    async fn drain_shutdown(&self) -> Result<(), String> {
        self.control.wait_for_permits().await;
        let router = Arc::clone(&self.router);
        tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
                .begin_shutdown_after_recovery()
                .map_err(|_| "ordinary lifecycle shutdown was rejected".to_owned())
        })
        .await
        .map_err(|_| "ordinary lifecycle shutdown task failed".to_owned())??;
        self.drive_until_idle().await?;
        let router = Arc::clone(&self.router);
        let stopped = tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())
                .map(|router| router.shutdown_complete())
        })
        .await
        .map_err(|_| "ordinary lifecycle shutdown task failed".to_owned())??;
        stopped
            .then_some(())
            .ok_or_else(|| "ordinary lifecycle shutdown was not proven by its owner".to_owned())
    }
}

#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_shutdown_tests.rs"]
mod ordinary_lifecycle_cadence_shutdown_tests;
#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_tests.rs"]
mod ordinary_lifecycle_cadence_tests;
