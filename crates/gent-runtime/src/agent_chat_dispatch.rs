//! Pure authority-gated access to the durable agent-chat provider-dispatch outbox.

use gent_ports::AgentChatPromptDispatchLedger;
use gent_types::{AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, HostEpoch};

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

    pub fn claim_excluding_runs(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
        excluded_run_ids: &[AgentChatRunId],
    ) -> Result<AgentChatPromptDispatchResult, RuntimeError> {
        if self.authority != AgentChatPromptDispatchAuthority::Approved {
            return Ok(AgentChatPromptDispatchResult::DeniedObserver);
        }
        Ok(self
            .ledger
            .claim_agent_chat_prompt_dispatch_excluding_runs(
                coordinator_id,
                host_epoch,
                provider,
                excluded_run_ids,
            )?
            .map_or(AgentChatPromptDispatchResult::Empty, |saved| {
                AgentChatPromptDispatchResult::Claimed(Box::new(saved))
            }))
    }

    /// Marks the durable boundary immediately before a provider invocation may occur.
    ///
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

    pub fn fail_prelaunch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        error: &str,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.fail_agent_chat_prompt_prelaunch(
                message_id,
                coordinator_id,
                host_epoch,
                error,
            )?;
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

#[path = "agent_chat_dispatch_terminal.rs"]
mod terminal;

#[cfg(test)]
#[path = "agent_chat_dispatch_tests.rs"]
mod tests;
