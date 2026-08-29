use crate::public_driver_runtime::PublicDriversRuntime;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    PublicProviderRunError, TranscriptLedger,
};
use gent_runtime::{AgentChatPromptDispatchResult, RuntimeError};
use gent_types::{AgentChatPromptSaved, DurableTurnPhase, HostEpoch};
use std::collections::BTreeMap;
use std::sync::Arc;
mod execution;
mod interrupt;
mod permission;
#[path = "codex_prompt_lifecycle_phase.rs"]
mod phase;
mod record;
mod scheduler;
mod start;
mod summary;
pub(crate) use execution::CodexPromptExecution;
pub(crate) use summary::CodexSummaryHook;
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexPromptDispatchOutcome {
    Denied,
    Busy,
    Empty,
    Started { run_id: String },
    Unprovable { run_id: String },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CodexPromptPoll {
    pub facts: u16,
    pub exited: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Binding {
    prompt: AgentChatPromptSaved,
    sequence: u64,
    settled: bool,
    releasing: bool,
}
#[derive(Debug)]
pub(crate) struct CodexPromptLifecycle<L, D, R> {
    runtime: PublicDriversRuntime<L, D, R>,
    runner: D,
    coordinator_id: String,
    active: BTreeMap<String, Binding>,
    summary_hook: Option<Arc<dyn CodexSummaryHook>>,
}
impl<L, D, R> CodexPromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    #[must_use]
    pub(crate) fn new(runtime: PublicDriversRuntime<L, D, R>, coordinator_id: String) -> Self {
        let runner = runtime.runner();
        Self {
            runtime,
            runner,
            coordinator_id,
            active: BTreeMap::new(),
            summary_hook: None,
        }
    }

    pub(crate) fn with_summary_hook(mut self, hook: Arc<dyn CodexSummaryHook>) -> Self {
        self.summary_hook = Some(hook);
        self
    }
    pub(crate) fn dispatch_next(
        &mut self,
        host_epoch: HostEpoch,
    ) -> Result<CodexPromptDispatchOutcome, RuntimeError> {
        let excluded_run_ids = self
            .active
            .keys()
            .filter(|run_id| {
                self.active
                    .get(*run_id)
                    .is_some_and(|binding| !binding.settled || binding.releasing)
            })
            .cloned()
            .map(gent_types::AgentChatRunId)
            .collect::<Vec<_>>();
        match self.runtime.claim_prompt_excluding_runs(
            &self.coordinator_id,
            host_epoch,
            gent_types::AgentChatProvider::Codex,
            &excluded_run_ids,
        )? {
            AgentChatPromptDispatchResult::DeniedObserver => Ok(CodexPromptDispatchOutcome::Denied),
            AgentChatPromptDispatchResult::Empty => Ok(CodexPromptDispatchOutcome::Empty),
            AgentChatPromptDispatchResult::Claimed(prompt) => {
                let refresh = self
                    .runner
                    .refresh_codex_mcp_config(&prompt.run_id.0)
                    .map_err(RuntimeError::from)?;
                if refresh {
                    self.active.remove(&prompt.run_id.0);
                }
                let active_run = self.active.get(&prompt.run_id.0);
                let reuses_settled_session = active_run.is_some_and(|binding| binding.settled)
                    && self.runner.has_codex_session(&prompt.run_id.0);
                let other_settled_runs = self
                    .active
                    .iter()
                    .filter(|(run_id, binding)| {
                        run_id.as_str() != prompt.run_id.0.as_str()
                            && binding.settled
                            && !binding.releasing
                            && self.runner.has_codex_session(run_id)
                    })
                    .map(|(run_id, _)| run_id.clone())
                    .collect::<Vec<_>>();
                if active_run.is_some() && !reuses_settled_session {
                    self.runtime.release_prompt_claim(
                        &prompt.message.message_id,
                        &self.coordinator_id,
                        host_epoch,
                    )?;
                    return Ok(CodexPromptDispatchOutcome::Busy);
                }
                for run_id in &other_settled_runs {
                    self.runner.release_codex_session(run_id)?;
                    self.active.remove(run_id);
                }
                start::prompt(
                    &self.runtime,
                    &self.runner,
                    &self.coordinator_id,
                    &mut self.active,
                    *prompt,
                    host_epoch,
                    refresh,
                )
            }
        }
    }
    pub(crate) fn has_settled_session(&self) -> bool {
        self.active.iter().any(|(run_id, binding)| {
            binding.settled && !binding.releasing && self.runner.has_codex_session(run_id)
        })
    }

    pub(crate) fn poll(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<CodexPromptPoll>, RuntimeError> {
        let effects = self.runner.poll_codex_prompt(run_id).map_err(|_| {
            RuntimeError::ProviderRun(PublicProviderRunError::Failed(
                "provider poll unavailable".into(),
            ))
        })?;
        let Some(effects) = effects else {
            return Ok(None);
        };
        let mut facts: u16 = 0;
        let mut terminal = None;
        for effect in effects {
            match effect {
                CodexRunnerEffect::Fact(fact) => {
                    terminal = terminal.or_else(|| phase::terminal(&fact));
                    self.record_wire(run_id, host_epoch, &fact)?;
                    facts += 1;
                }
                CodexRunnerEffect::ControlRequest(request) => {
                    facts = facts.saturating_add(
                        self.record_permission_request(run_id, host_epoch, request)?,
                    );
                }
                CodexRunnerEffect::Exited { code } => {
                    self.record_exit(run_id, host_epoch, code)?;
                    self.settle_if_open(run_id, host_epoch, DurableTurnPhase::Failed)?;
                    self.active.remove(run_id);
                    return Ok(Some(CodexPromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        if let Some(phase) = terminal {
            self.settle_if_open(run_id, host_epoch, phase)?;
            if phase == DurableTurnPhase::Completed {
                if let Some(binding) = self.active.get(run_id) {
                    if let Some(hook) = &self.summary_hook {
                        let _ = hook.schedule(&binding.prompt.message.conversation_id);
                    }
                }
            }
            if phase != DurableTurnPhase::Completed {
                self.release_failed_session(run_id)?;
            }
        }
        Ok(Some(CodexPromptPoll {
            facts,
            exited: false,
        }))
    }

    fn settle_if_open(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if !binding.settled {
            self.runtime.settle_prompt_terminal(
                &binding.prompt.message.message_id,
                &self.coordinator_id,
                host_epoch,
                phase,
            )?;
            binding.settled = true;
        }
        Ok(())
    }

    fn release_failed_session(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if binding.releasing {
            return Ok(());
        }
        self.runner.signal_codex_process(
            run_id,
            gent_drivers::interrupt::ProcessTreeSignal::Terminate,
        )?;
        binding.releasing = true;
        Ok(())
    }
}
#[path = "codex_prompt_lifecycle_error.rs"]
mod error;
use error::missing_binding;
