use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

use gent_types::{AgentChatPromptDisposition, AgentChatProvider};

use super::PromptWake;

struct Waiter {
    prompt: PromptWake,
    resource: String,
}

#[derive(Default)]
struct Admissions {
    waiters: BTreeMap<String, Waiter>,
}

#[derive(Clone, Default)]
pub(super) struct AdmissionWaiters(Arc<Mutex<Admissions>>);

pub(super) fn model(model_id: &str) -> String {
    format!("model:{model_id}")
}

pub(super) fn provider(provider: AgentChatProvider) -> String {
    format!("provider:{provider:?}")
}

impl AdmissionWaiters {
    pub(super) fn wait(&self, prompt: &PromptWake, resource: &str) -> Result<(), String> {
        self.lock()?.waiters.insert(
            prompt.receipt_id.0.clone(),
            Waiter {
                prompt: prompt.clone(),
                resource: resource.to_owned(),
            },
        );
        Ok(())
    }

    pub(super) fn is_waiting(&self, prompt: &PromptWake) -> bool {
        self.lock()
            .is_ok_and(|state| state.waiters.contains_key(&prompt.receipt_id.0))
    }

    pub(super) fn stop_waiting(&self, prompt: &PromptWake) -> bool {
        self.lock()
            .is_ok_and(|mut state| state.waiters.remove(&prompt.receipt_id.0).is_some())
    }

    pub(super) fn held_for_run(&self, run_id: &str) -> Result<Vec<(PromptWake, String)>, String> {
        Ok(self
            .lock()?
            .waiters
            .values()
            .filter(|waiter| {
                waiter.prompt.run_id.0 == run_id
                    && waiter.prompt.disposition == AgentChatPromptDisposition::Send
            })
            .map(|waiter| (waiter.prompt.clone(), waiter.resource.clone()))
            .collect())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Admissions>, String> {
        self.0
            .lock()
            .map_err(|_| "prompt admission registry is unavailable".to_owned())
    }
}

#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_standalone_install_hold_tests.rs"]
mod install_hold_tests;
#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_standalone_waiters_tests.rs"]
mod tests;
