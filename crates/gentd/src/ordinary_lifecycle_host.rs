use crate::provider_lifecycle_host::{ProviderLifecycleHost, ProviderLifecycleWakePort};
use gent_types::AgentChatProvider;

pub(crate) trait OrdinaryLifecycleHost: Send {
    fn provider(&self) -> AgentChatProvider;
    fn arm_authority_recovery(&mut self) -> Result<(), ()>;
    fn wake(&mut self) -> Result<(), ()>;
    fn drive(&mut self) -> Result<(), ()>;
    fn needs_drive(&self) -> bool;
    fn respond_claude_permission(
        &self,
        _run_id: &str,
        _request_id: &str,
        _behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        _persist_suggestions: bool,
    ) -> Result<(), ()> {
        Err(())
    }
    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), ()> {
        let _ = updated_input;
        self.respond_claude_permission(run_id, request_id, behavior, persist_suggestions)
    }
    fn respond_codex_permission(
        &self,
        _run_id: &str,
        _request_id: &str,
        _decision: gent_drivers::codex_control::CodexControlDecision,
        _answers: Option<serde_json::Value>,
    ) -> Result<(), ()> {
        Err(())
    }
    fn interrupt_run(&mut self, _: &str) -> Result<(), ()> {
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
    O: crate::private_lifecycle_loop::PrivateLifecycleOwner + Send,
    O::Error: std::fmt::Debug,
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
        self.host.drive().map(|_| ()).map_err(|error| {
            eprintln!(
                "{:#?} provider lifecycle drive failed: {error:?}",
                self.provider
            );
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
    ) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
        self.host
            .respond_codex_permission(run_id, request_id, decision, answers)
    }

    fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.host.interrupt_run(run_id)
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
