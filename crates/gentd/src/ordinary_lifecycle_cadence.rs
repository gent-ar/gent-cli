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
use crate::ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter;

const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Prompt-commit adapter that arms a durable-selection host before notifying the cadence.
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryPromptWake<L> {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    notify: Arc<Notify>,
}

/// One daemon-owned cadence paired with [`OrdinaryPromptWake`].
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryLifecycleCadence<L> {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
    notify: Arc<Notify>,
}

/// Creates the only prompt wake and cadence pair for one ordinary lifecycle router.
#[must_use]
pub(crate) fn pair<L>(
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<L>>>,
) -> (OrdinaryPromptWake<L>, OrdinaryLifecycleCadence<L>) {
    let notify = Arc::new(Notify::new());
    (
        OrdinaryPromptWake {
            router: Arc::clone(&router),
            notify: Arc::clone(&notify),
        },
        OrdinaryLifecycleCadence { router, notify },
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

impl<L: Send + 'static> OrdinaryLifecycleCadence<L> {
    /// Runs durable recovery once, then waits without polling until a committed prompt wakes it.
    pub(crate) async fn run(self) {
        if self.activate_recovery().await.is_ok() {
            self.drive_until_idle().await;
        }
        loop {
            self.notify.notified().await;
            self.drive_until_idle().await;
        }
    }

    async fn activate_recovery(&self) -> Result<(), ()> {
        let router = Arc::clone(&self.router);
        tokio::task::spawn_blocking(move || {
            router
                .lock()
                .map_err(|_| ())?
                .activate_recovery()
                .map_err(|_| ())
        })
        .await
        .map_err(|_| ())?
    }

    async fn drive_until_idle(&self) {
        loop {
            let router = Arc::clone(&self.router);
            let active = tokio::task::spawn_blocking(move || {
                router.lock().map_err(|_| ())?.drive_once().map_err(|_| ())
            })
            .await;
            let Ok(Ok(active)) = active else {
                tokio::time::sleep(ACTIVE_POLL_INTERVAL).await;
                continue;
            };
            if !active {
                return;
            }
            tokio::time::sleep(ACTIVE_POLL_INTERVAL).await;
        }
    }
}

#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_tests.rs"]
mod ordinary_lifecycle_cadence_tests;
