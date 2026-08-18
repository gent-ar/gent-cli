//! Process adapter implemented only by the daemon-owned Codex prompt runner.

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
    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
    ) -> Result<(), PublicProviderRunError>;
    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
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

    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
    ) -> Result<(), PublicProviderRunError> {
        self.submit(run_id, prompt, goal)
    }

    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.signal_process(run_id, signal)
    }
}
