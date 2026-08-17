//! Pure authority-gated access to the durable agent-chat provider-dispatch outbox.

use gent_ports::AgentChatPromptDispatchLedger;
use gent_types::{AgentChatPromptSaved, AgentChatProvider, HostEpoch};

use crate::RuntimeError;

/// Explicit authority required before a daemon may claim provider-bound prompts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatPromptDispatchAuthority {
    /// Observer mode neither reads nor changes the durable dispatch outbox.
    #[default]
    Observer,
    /// A separately approved daemon-owned lifecycle may claim and settle prompts.
    Approved,
}

/// A no-read observer denial, no work, or one exclusively claimed durable prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatPromptDispatchResult {
    DeniedObserver,
    Empty,
    Claimed(Box<AgentChatPromptSaved>),
}

/// Delegates durable dispatch ownership to a port without knowing providers or processes.
#[derive(Clone, Debug)]
pub struct AgentChatPromptDispatchService<L> {
    ledger: L,
    authority: AgentChatPromptDispatchAuthority,
}

impl<L> AgentChatPromptDispatchService<L> {
    /// Builds an inert observer service unless daemon lifecycle authority is explicitly approved.
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatPromptDispatchAuthority) -> Self {
        Self { ledger, authority }
    }
}
impl<L: AgentChatPromptDispatchLedger> AgentChatPromptDispatchService<L> {
    /// Claims one durable provider-bound prompt for the supplied daemon ownership fence.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches durable claim validation.
    pub fn claim(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<AgentChatPromptDispatchResult, RuntimeError> {
        if self.authority != AgentChatPromptDispatchAuthority::Approved {
            return Ok(AgentChatPromptDispatchResult::DeniedObserver);
        }
        Ok(self
            .ledger
            .claim_agent_chat_prompt_dispatch(coordinator_id, host_epoch, provider)?
            .map_or(AgentChatPromptDispatchResult::Empty, |saved| {
                AgentChatPromptDispatchResult::Claimed(Box::new(saved))
            }))
    }

    /// Marks the durable boundary immediately before a provider invocation may occur.
    ///
    /// # Errors
    /// Returns an error when approved durable owner validation rejects the release.
    pub fn begin_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger
                .begin_agent_chat_prompt_launch(message_id, coordinator_id, host_epoch)?;
        }
        Ok(())
    }

    /// Confirms a launch only after the daemon-owned runner reports it successfully started.
    ///
    /// # Errors
    /// Returns an error when approved durable owner validation rejects settlement.
    pub fn confirm_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.confirm_agent_chat_prompt_started(
                message_id,
                coordinator_id,
                host_epoch,
            )?;
        }
        Ok(())
    }

    /// Releases a claim before a runner launch boundary has been crossed.
    /// # Errors
    /// Returns an error when the durable claim is no longer owned by this daemon.
    pub fn release_claim(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger
                .release_agent_chat_prompt_claim(message_id, coordinator_id, host_epoch)?;
        }
        Ok(())
    }

    /// Releases a launch marker only after a local result proves no runner was invoked.
    /// # Errors
    /// Returns an error when the durable launch marker is no longer owned by this daemon.
    pub fn release_unstarted_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.release_agent_chat_prompt_unstarted_launch(
                message_id,
                coordinator_id,
                host_epoch,
            )?;
        }
        Ok(())
    }

    /// Retires an ambiguous launch without allowing it to be automatically replayed.
    /// # Errors
    /// Returns an error when the durable launch marker is no longer owned by this daemon.
    pub fn mark_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.mark_agent_chat_prompt_unprovable(
                message_id,
                coordinator_id,
                host_epoch,
            )?;
        }
        Ok(())
    }

    /// Settles a provider-owned prompt after its daemon lifecycle has reached a terminal state.
    /// # Errors
    /// Returns an error when the durable started marker is no longer owned by this daemon.
    pub fn settle(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.settle_agent_chat_prompt_dispatch(
                message_id,
                coordinator_id,
                host_epoch,
            )?;
        }
        Ok(())
    }

    /// Recovers only known-prelaunch work after a successor has fenced the old epoch.
    /// # Errors
    /// Returns an error when the supplied epoch is not an open successor epoch.
    pub fn recover(&self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger
                .recover_agent_chat_prompt_dispatches(host_epoch)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gent_ports::{AgentChatPromptDispatchLedger, LedgerError};
    use gent_types::{AgentChatPromptSaved, AgentChatProvider, HostEpoch};

    use super::{
        AgentChatPromptDispatchAuthority, AgentChatPromptDispatchResult,
        AgentChatPromptDispatchService,
    };

    #[derive(Clone, Default)]
    struct Ledger(Arc<Mutex<u8>>);

    impl AgentChatPromptDispatchLedger for Ledger {
        fn claim_agent_chat_prompt_dispatch(
            &self,
            _: &str,
            _: HostEpoch,
            _: AgentChatProvider,
        ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
            *self.0.lock().unwrap() += 1;
            Ok(None)
        }

        fn begin_agent_chat_prompt_launch(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not launch")
        }

        fn confirm_agent_chat_prompt_started(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not confirm")
        }

        fn release_agent_chat_prompt_claim(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not release")
        }

        fn release_agent_chat_prompt_unstarted_launch(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not release")
        }

        fn mark_agent_chat_prompt_unprovable(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not mark")
        }

        fn settle_agent_chat_prompt_dispatch(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not settle")
        }

        fn recover_agent_chat_prompt_dispatches(&self, _: HostEpoch) -> Result<(), LedgerError> {
            unreachable!("observer test must not recover")
        }
    }

    #[test]
    fn observer_does_not_read_or_claim_provider_bound_prompts() {
        let ledger = Ledger::default();
        let service = AgentChatPromptDispatchService::new(
            ledger.clone(),
            AgentChatPromptDispatchAuthority::Observer,
        );
        assert_eq!(
            service
                .claim("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
                .unwrap(),
            AgentChatPromptDispatchResult::DeniedObserver
        );
        assert_eq!(*ledger.0.lock().unwrap(), 0);
    }

    #[test]
    fn approved_service_exposes_only_an_exclusive_durable_claim() {
        let ledger = Ledger::default();
        let service = AgentChatPromptDispatchService::new(
            ledger.clone(),
            AgentChatPromptDispatchAuthority::Approved,
        );
        assert_eq!(
            service
                .claim("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
                .unwrap(),
            AgentChatPromptDispatchResult::Empty
        );
        assert_eq!(*ledger.0.lock().unwrap(), 1);
    }
}
