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

    /// Releases a failed-to-start prompt without exposing provider state to this runtime layer.
    ///
    /// # Errors
    /// Returns an error when approved durable owner validation rejects the release.
    pub fn release(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.release_agent_chat_prompt_dispatch(
                message_id,
                coordinator_id,
                host_epoch,
            )?;
        }
        Ok(())
    }

    /// Settles a provider-owned prompt after its daemon lifecycle has reached a terminal state.
    ///
    /// # Errors
    /// Returns an error when approved durable owner validation rejects settlement.
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

        fn release_agent_chat_prompt_dispatch(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not release")
        }

        fn settle_agent_chat_prompt_dispatch(
            &self,
            _: &str,
            _: &str,
            _: HostEpoch,
        ) -> Result<(), LedgerError> {
            unreachable!("observer test must not settle")
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
