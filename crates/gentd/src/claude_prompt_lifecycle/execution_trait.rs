use super::super::ClaudePromptExecution;
use super::{ClaudePromptRunner, ClaudePromptStart};
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::supervisor::{ProcessLauncher, ProviderProcess};
use gent_ports::PublicProviderRunError;

impl<L, P> ClaudePromptExecution for ClaudePromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess + Send,
{
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare(run_id, prompt)
    }
    fn cancel_claude_prompt(&self, run_id: &str) {
        self.cancel(run_id);
        self.cleanup_config(run_id);
    }
    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        self.poll(run_id)
    }
    fn signal_claude_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        let result = super::lock(&self.runner)
            .signal(run_id, signal)
            .map_err(super::map_error);
        if result.is_ok() {
            self.cleanup_config(run_id);
        }
        result
    }
    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_permission(run_id, request_id, behavior, persist_suggestions)
    }
    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            updated_input,
        )
    }
}
