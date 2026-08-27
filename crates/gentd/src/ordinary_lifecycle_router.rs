use gent_ports::AgentChatReadLedger;
use gent_runtime::AgentChatReadService;
use gent_types::AgentChatProvider;

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
pub(crate) use crate::ordinary_lifecycle_host::{OrdinaryLifecycleHost, OrdinaryProviderHost};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrdinaryLifecycleRouterError {
    DurableSelectionUnavailable,
    HostUnavailable(AgentChatProvider),
    DuplicateHost(AgentChatProvider),
}

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

    pub(crate) fn activate_recovery(&mut self) -> Result<(), OrdinaryLifecycleRouterError> {
        for host in &mut self.hosts {
            host.arm_authority_recovery()
                .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(host.provider()))?;
        }
        Ok(())
    }

    pub(crate) fn begin_shutdown_after_recovery(
        &mut self,
    ) -> Result<(), OrdinaryLifecycleRouterError> {
        for host in &mut self.hosts {
            host.begin_shutdown_after_recovery()
                .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(host.provider()))?;
        }
        Ok(())
    }

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

    #[must_use]
    pub(crate) fn shutdown_complete(&self) -> bool {
        self.hosts.iter().all(|host| host.shutdown_complete())
    }

    pub(crate) fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), OrdinaryLifecycleRouterError> {
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
    ) -> Result<(), OrdinaryLifecycleRouterError> {
        self.hosts
            .iter()
            .find(|host| host.provider() == AgentChatProvider::Claude)
            .ok_or(OrdinaryLifecycleRouterError::HostUnavailable(
                AgentChatProvider::Claude,
            ))?
            .respond_claude_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(AgentChatProvider::Claude))
    }

    pub(crate) fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), OrdinaryLifecycleRouterError> {
        self.hosts
            .iter()
            .find(|host| host.provider() == AgentChatProvider::Codex)
            .ok_or(OrdinaryLifecycleRouterError::HostUnavailable(
                AgentChatProvider::Codex,
            ))?
            .respond_codex_permission(run_id, request_id, decision, answers)
            .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(AgentChatProvider::Codex))
    }

    pub(crate) fn interrupt_run(
        &mut self,
        provider: AgentChatProvider,
        run_id: &str,
    ) -> Result<(), OrdinaryLifecycleRouterError> {
        self.hosts
            .iter_mut()
            .find(|host| host.provider() == provider)
            .ok_or(OrdinaryLifecycleRouterError::HostUnavailable(provider))?
            .interrupt_run(run_id)
            .map_err(|()| OrdinaryLifecycleRouterError::HostUnavailable(provider))
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
