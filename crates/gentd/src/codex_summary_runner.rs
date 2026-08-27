use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use crate::codex_prompt_lifecycle::CodexSummaryHook;
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::{
    buffering::BufferPolicy,
    codex_runner::{CodexAppServerRunner, CodexRunStart, CodexRunnerEffect},
    codex_session::{CodexSessionConfig, CodexTurnOptions},
    process::SystemLauncher,
    supervisor::ProcessLauncher,
};
use gent_ports::{ConversationSummaryRunner, PortError};
use gent_runtime::conversation_summary_scheduler::ConversationSummaryScheduler;
use gent_types::{NormalizedProviderEvent, RunVersionLock, SandboxWorkspaceAccess};

const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_POLL_TIME: Duration = Duration::from_secs(60);
const POLL_DELAY: Duration = Duration::from_millis(5);

#[derive(Clone, Debug)]
pub(crate) struct CodexSummaryRunner<L = SystemLauncher> {
    launcher: L,
    lock: RunVersionLock,
    workspace_root: PathBuf,
}

impl<L> CodexSummaryRunner<L> {
    pub(crate) fn new(launcher: L, lock: RunVersionLock, workspace_root: PathBuf) -> Self {
        Self {
            launcher,
            lock,
            workspace_root,
        }
    }
}

impl<L> ConversationSummaryRunner for CodexSummaryRunner<L>
where
    L: ProcessLauncher + Clone + Send + Sync,
{
    fn run_summary(
        &self,
        provider: &str,
        model_version: &str,
        prompt: &str,
    ) -> Result<String, PortError> {
        if provider != "codex" || self.lock.provider != "codex" {
            return Err(PortError::Unavailable(
                "Codex summary runner received another provider".into(),
            ));
        }
        if !self.workspace_root.is_absolute() {
            return Err(PortError::Unavailable(
                "Codex summary workspace is not absolute".into(),
            ));
        }
        let options = CodexTurnOptions::summary(model_version)
            .map_err(|error| PortError::Provider(error.to_string()))?;
        let session = CodexSessionConfig {
            working_directory: Some(self.workspace_root.to_string_lossy().into_owned()),
            resume_thread_id: None,
            turn_options: options,
            mcp_servers: None,
        };
        let run_id = format!("summary-{}", uuid::Uuid::new_v4());
        let mut runner = CodexAppServerRunner::new(
            self.launcher.clone(),
            BufferPolicy::new(16, MAX_OUTPUT_BYTES, 0, 0)
                .map_err(|error| PortError::Provider(error.to_string()))?,
        );
        runner
            .start(CodexRunStart {
                run_id: run_id.clone(),
                lock: self.lock.clone(),
                session,
                workspace_root: self.workspace_root.clone(),
                workspace_access: SandboxWorkspaceAccess::ReadOnly,
                prompt: prompt.into(),
                goal: None,
                attachments: Vec::new(),
            })
            .map_err(|error| PortError::Provider(error.to_string()))?;
        let result = collect(&mut runner, &run_id);
        let _ = runner.terminate(&run_id);
        result
    }
}

#[derive(Debug)]
pub(crate) struct CodexSummarySchedulerHook<L = SystemLauncher> {
    scheduler: ConversationSummaryScheduler<gent_store::SqliteLedger, CodexSummaryRunner<L>>,
}

impl<L> CodexSummarySchedulerHook<L>
where
    L: ProcessLauncher + Clone + Send + Sync + std::fmt::Debug,
{
    pub(crate) fn new(ledger: gent_store::SqliteLedger, runner: CodexSummaryRunner<L>) -> Self {
        Self {
            scheduler: ConversationSummaryScheduler::new(ledger, runner),
        }
    }
}

impl<L> CodexSummaryHook for CodexSummarySchedulerHook<L>
where
    L: ProcessLauncher + Clone + Send + Sync + std::fmt::Debug + 'static,
{
    fn schedule(&self, conversation_id: &str) -> Result<(), gent_runtime::RuntimeError> {
        self.scheduler.schedule(conversation_id).map(|_| ())
    }
}

fn collect<L, P>(runner: &mut CodexAppServerRunner<L, P>, run_id: &str) -> Result<String, PortError>
where
    L: ProcessLauncher<Process = P>,
    P: gent_drivers::supervisor::ProviderProcess,
{
    let deadline = Instant::now() + MAX_POLL_TIME;
    let mut output = String::new();
    loop {
        if Instant::now() >= deadline {
            return Err(PortError::Unavailable("Codex summary timed out".into()));
        }
        if let Some(effects) = runner
            .poll(run_id)
            .map_err(|error| PortError::Provider(error.to_string()))?
        {
            for effect in effects {
                match effect {
                    CodexRunnerEffect::Fact(PublicWireFact::Event(
                        NormalizedProviderEvent::Output { text, .. },
                    )) => {
                        if output.len() + text.len() > MAX_OUTPUT_BYTES {
                            return Err(PortError::Provider(
                                "Codex summary output exceeded its bound".into(),
                            ));
                        }
                        output.push_str(&text);
                    }
                    CodexRunnerEffect::Fact(PublicWireFact::Event(
                        NormalizedProviderEvent::TurnEnded { .. },
                    )) => {
                        if output.trim().is_empty() {
                            return Err(PortError::Provider(
                                "Codex summary returned no text".into(),
                            ));
                        }
                        return Ok(output);
                    }
                    CodexRunnerEffect::Exited { .. } => {
                        return Err(PortError::Provider(
                            "Codex summary process exited before completion".into(),
                        ));
                    }
                    CodexRunnerEffect::Fact(_) | CodexRunnerEffect::ControlRequest(_) => {}
                }
            }
        } else {
            thread::sleep(POLL_DELAY);
        }
    }
}
