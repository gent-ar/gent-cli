//! Process adapter implemented only by the daemon-owned Codex prompt runner.

use gent_drivers::codex_control::CodexControlDecision;
use gent_drivers::codex_prompt_runner::{CodexPromptRunner, CodexPromptStart};
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::supervisor::{ProcessLauncher, ProviderProcess};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::GoalProjection;

pub(crate) trait CodexPromptExecution: PublicProviderRunner {
    fn prepare_codex_prompt(
        &self,
        run_id: String,
        prompt: CodexPromptStart,
    ) -> Result<(), PublicProviderRunError>;
    fn cancel_codex_prompt(&self, run_id: &str);
    fn poll_codex_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError>;
    fn has_codex_session(&self, run_id: &str) -> bool;
    fn release_codex_session(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
    fn refresh_codex_mcp_config(&self, run_id: &str) -> Result<bool, PublicProviderRunError>;
    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError>;
    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError>;
    fn interrupt_codex_turn(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
    fn respond_codex_control(
        &self,
        run_id: &str,
        request_id: &str,
        decision: CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError>;
}

impl<L, P> CodexPromptExecution for CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess + Send,
{
    fn prepare_codex_prompt(
        &self,
        run_id: String,
        prompt: CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare(run_id, prompt)
    }

    fn cancel_codex_prompt(&self, run_id: &str) {
        self.cancel(run_id);
    }

    fn poll_codex_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError> {
        self.poll(run_id)
    }

    fn has_codex_session(&self, run_id: &str) -> bool {
        self.owns(run_id)
    }

    fn release_codex_session(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.release_session(run_id)
    }

    fn refresh_codex_mcp_config(&self, run_id: &str) -> Result<bool, PublicProviderRunError> {
        self.refresh_mcp_config(run_id)
    }

    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        self.submit(run_id, prompt, goal, attachments)
    }

    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.signal_process(run_id, signal)
    }

    fn interrupt_codex_turn(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.interrupt_turn(run_id)
    }

    fn respond_codex_control(
        &self,
        run_id: &str,
        request_id: &str,
        decision: CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_control(run_id, request_id, decision, answers)
    }
}
