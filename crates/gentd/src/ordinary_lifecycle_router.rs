//! Private router for the future ordinary Claude/Codex terminal authority.
//!
//! The router owns no durable provider state. It resolves a committed prompt's
//! provider from the canonical run selection, then only arms the corresponding
//! pre-approved host. A later daemon cadence drives each bounded host once.

use gent_ports::AgentChatReadLedger;
use gent_runtime::AgentChatReadService;
use gent_types::AgentChatProvider;

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::provider_lifecycle_host::{ProviderLifecycleHost, ProviderLifecycleWakePort};

/// Minimal private control surface shared by one already-approved provider host.
pub(crate) trait OrdinaryLifecycleHost: Send {
    fn provider(&self) -> AgentChatProvider;
    fn arm_authority_recovery(&mut self) -> Result<(), ()>;
    fn wake(&mut self) -> Result<(), ()>;
    fn drive(&mut self) -> Result<(), ()>;
    fn needs_drive(&self) -> bool;
    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        Err(())
    }
    fn escalate_shutdown(&mut self) -> Result<(), ()> {
        Err(())
    }
    fn shutdown_complete(&self) -> bool {
        false
    }
}

/// Names one bounded provider owner for the ordinary daemon composition.
///
/// This is only a live-process adapter: the wrapped lifecycle host still obtains pending work,
/// recovery, session bindings, and terminal facts from the durable ledger.
#[derive(Debug)]
pub(crate) struct OrdinaryProviderHost<O> {
    provider: AgentChatProvider,
    host: ProviderLifecycleHost<O>,
}

impl<O> OrdinaryProviderHost<O> {
    /// Retains one already-approved bounded lifecycle owner for its exact provider.
    #[must_use]
    pub(crate) const fn new(provider: AgentChatProvider, host: ProviderLifecycleHost<O>) -> Self {
        Self { provider, host }
    }
}

impl<O> OrdinaryLifecycleHost for OrdinaryProviderHost<O>
where
    O: crate::private_lifecycle_loop::PrivateLifecycleOwner + Send,
{
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }

    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        self.host
            .arm_authority_recovery()
            .map(|_| ())
            .map_err(|_| ())
    }

    fn wake(&mut self) -> Result<(), ()> {
        ProviderLifecycleWakePort::wake_after_prompt_commit(&mut self.host)
            .map(|_| ())
            .map_err(|_| ())
    }

    fn drive(&mut self) -> Result<(), ()> {
        self.host.drive().map(|_| ()).map_err(|_| ())
    }

    fn needs_drive(&self) -> bool {
        self.host.is_armed()
    }

    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        self.host.begin_shutdown_after_recovery().map_err(|_| ())
    }

    fn escalate_shutdown(&mut self) -> Result<(), ()> {
        self.host.escalate_shutdown().map_err(|_| ())
    }

    fn shutdown_complete(&self) -> bool {
        self.host.shutdown_complete()
    }
}

/// Fail-closed routing result; it never contains provider-native diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrdinaryLifecycleRouterError {
    DurableSelectionUnavailable,
    HostUnavailable(AgentChatProvider),
    DuplicateHost(AgentChatProvider),
}

/// Composition-owned lifecycle routing for ordinary terminal prompts.
pub(crate) struct OrdinaryPublicLifecycleRouter<L> {
    reads: AgentChatReadService<L>,
    hosts: Vec<Box<dyn OrdinaryLifecycleHost>>,
}

impl<L> std::fmt::Debug for OrdinaryPublicLifecycleRouter<L> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OrdinaryPublicLifecycleRouter")
            .field("host_count", &self.hosts.len())
            .finish_non_exhaustive()
    }
}

impl<L> OrdinaryPublicLifecycleRouter<L> {
    /// Binds each approved host exactly once without driving or waking it.
    pub(crate) fn new(
        reads: AgentChatReadService<L>,
        hosts: Vec<Box<dyn OrdinaryLifecycleHost>>,
    ) -> Result<Self, OrdinaryLifecycleRouterError> {
        for (index, host) in hosts.iter().enumerate() {
            if hosts[..index]
                .iter()
                .any(|previous| previous.provider() == host.provider())
            {
                return Err(OrdinaryLifecycleRouterError::DuplicateHost(host.provider()));
            }
        }
        Ok(Self { reads, hosts })
    }

    /// Drives every approved host at most once and reports whether another demand-driven pass is needed.
    pub(crate) fn drive_once(&mut self) -> Result<bool, OrdinaryLifecycleRouterError> {
        let mut needs_drive = false;
        for host in &mut self.hosts {
            if !host.needs_drive() {
                continue;
            }
            host.drive()
                .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(host.provider()))?;
            needs_drive |= host.needs_drive();
        }
        Ok(needs_drive)
    }

    /// Arms every approved owner for one durable startup recovery pass.
    ///
    /// This has no provider side effect; a caller-controlled cadence drives the resulting wake.
    pub(crate) fn activate_recovery(&mut self) -> Result<(), OrdinaryLifecycleRouterError> {
        for host in &mut self.hosts {
            host.arm_authority_recovery()
                .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(host.provider()))?;
        }
        Ok(())
    }

    /// Queues graceful process-tree shutdown after every host has completed recovery.
    ///
    /// Hosts reject a fresh `AwaitingWake` state rather than fabricating a wake. The caller must
    /// keep driving the router until [`Self::shutdown_complete`] is true.
    pub(crate) fn begin_shutdown_after_recovery(
        &mut self,
    ) -> Result<(), OrdinaryLifecycleRouterError> {
        for host in &mut self.hosts {
            host.begin_shutdown_after_recovery()
                .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(host.provider()))?;
        }
        Ok(())
    }

    /// Queues one explicit shutdown escalation for every still-draining host.
    pub(crate) fn escalate_shutdown(&mut self) -> Result<(), OrdinaryLifecycleRouterError> {
        for host in &mut self.hosts {
            if host.shutdown_complete() {
                continue;
            }
            host.escalate_shutdown()
                .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(host.provider()))?;
        }
        Ok(())
    }

    /// Reports that every recovered host has drained after an explicit shutdown request.
    #[must_use]
    pub(crate) fn shutdown_complete(&self) -> bool {
        self.hosts.iter().all(|host| host.shutdown_complete())
    }
}

impl<L: AgentChatReadLedger> PromptCommitWake for OrdinaryPublicLifecycleRouter<L> {
    type Error = OrdinaryLifecycleRouterError;

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        let provider = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|_| OrdinaryLifecycleRouterError::DurableSelectionUnavailable)?
            .provider;
        self.hosts
            .iter_mut()
            .find(|host| host.provider() == provider)
            .ok_or(OrdinaryLifecycleRouterError::HostUnavailable(provider))?
            .wake()
            .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(provider))
    }
}

#[cfg(test)]
#[path = "ordinary_lifecycle_shutdown_tests.rs"]
mod ordinary_lifecycle_shutdown_tests;
