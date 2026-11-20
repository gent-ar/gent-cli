use crate::provider_lifecycle_host::{ProviderLifecycleHost, ProviderLifecycleWakePort};
use gent_types::AgentChatProvider;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Undelivered {
    NotPending,
    InvalidInput,
    PipeBroken,
}

impl Undelivered {
    pub(crate) const fn of(error: &gent_runtime::RuntimeError) -> Self {
        match error {
            gent_runtime::RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::NotActive,
            ) => Self::NotPending,
            gent_runtime::RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::PermissionInputInvalid(_),
            ) => Self::InvalidInput,
            _ => Self::PipeBroken,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DriveFailure {
    LedgerBusy(String),
    Failed(String),
}

impl DriveFailure {
    fn of(
        error: &crate::provider_lifecycle_host::ProviderLifecycleHostError<
            gent_runtime::RuntimeError,
        >,
    ) -> Self {
        match error {
            crate::provider_lifecycle_host::ProviderLifecycleHostError::Owner(
                gent_runtime::RuntimeError::Ledger(gent_ports::LedgerError::Busy(_)),
            ) => Self::LedgerBusy(format!("{error:?}")),
            _ => Self::Failed(format!("{error:?}")),
        }
    }
}

pub(crate) trait OrdinaryLifecycleHost: Send {
    fn provider(&self) -> AgentChatProvider;
    fn arm_authority_recovery(&mut self) -> Result<(), ()>;
    fn wake(&mut self) -> Result<(), ()>;
    fn drive(&mut self) -> Result<(), DriveFailure>;
    fn needs_drive(&self) -> bool;
    fn respond_claude_permission(
        &self,
        _run_id: &str,
        _request_id: &str,
        _behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        _persist_suggestions: bool,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        Err(crate::ordinary_lifecycle_host::Undelivered::NotPending)
    }
    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        let _ = updated_input;
        self.respond_claude_permission(run_id, request_id, behavior, persist_suggestions)
    }
    fn respond_codex_permission(
        &self,
        _run_id: &str,
        _request_id: &str,
        _decision: gent_drivers::codex_control::CodexControlDecision,
        _answers: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        Err(crate::ordinary_lifecycle_host::Undelivered::NotPending)
    }
    fn interrupt_run(&mut self, _: &str) -> Result<(), ()> {
        Err(())
    }
    fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, ()> {
        Ok(request.refused(gent_protocol::ProviderWarmOutcome::UnsupportedProvider))
    }
    fn interrupt_turn(&mut self, _: &str, _: &str) -> Result<(), ()> {
        Err(())
    }
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

#[derive(Debug)]
pub(crate) struct OrdinaryProviderHost<O> {
    provider: AgentChatProvider,
    host: ProviderLifecycleHost<O>,
}

impl<O> OrdinaryProviderHost<O> {
    #[must_use]
    pub(crate) const fn new(provider: AgentChatProvider, host: ProviderLifecycleHost<O>) -> Self {
        Self { provider, host }
    }
}

impl<O> OrdinaryLifecycleHost for OrdinaryProviderHost<O>
where
    O: crate::private_lifecycle_loop::PrivateLifecycleOwner<Error = gent_runtime::RuntimeError>
        + Send,
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

    fn drive(&mut self) -> Result<(), DriveFailure> {
        self.host.drive().map(|_| ()).map_err(|error| {
            eprintln!(
                "{:#?} provider lifecycle drive failed: {error:?}",
                self.provider
            );
            DriveFailure::of(&error)
        })
    }

    fn needs_drive(&self) -> bool {
        self.host.is_armed()
    }

    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        self.host.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            updated_input,
        )
    }

    fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        self.host
            .respond_codex_permission(run_id, request_id, decision, answers)
    }

    fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.host.interrupt_run(run_id)
    }
    fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, ()> {
        let outcome = self.host.tend_warm_pool(request)?;
        if outcome == Some(gent_protocol::ProviderWarmOutcome::Warming) {
            let _ = ProviderLifecycleWakePort::wake_after_prompt_commit(&mut self.host);
        }
        Ok(outcome)
    }
    fn interrupt_turn(&mut self, run_id: &str, turn_id: &str) -> Result<(), ()> {
        self.host.interrupt_turn(run_id, turn_id)
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
