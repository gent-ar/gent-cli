use crate::public_driver_runtime::PublicDriversRuntime;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::{AgentChatPromptDispatchResult, RuntimeError};
use gent_types::{AgentChatPromptSaved, DurableTurnPhase, HostEpoch};
use std::collections::BTreeMap;
use std::sync::Arc;
mod containment;
mod execution;
mod interrupt;
mod launch;
mod permission;
#[path = "codex_prompt_lifecycle_phase.rs"]
mod phase;
mod poll;
mod record;
mod recovery;
mod scheduler;
mod start;
mod steer;
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
    steers: Vec<AgentChatPromptSaved>,
    upgraded: bool,
}
impl Binding {
    fn idle(&self) -> bool {
        self.settled && !self.releasing && self.steers.is_empty()
    }
}
#[derive(Debug)]
pub(crate) struct CodexPromptLifecycle<L, D, R> {
    runtime: PublicDriversRuntime<L, D, R>,
    runner: D,
    coordinator_id: String,
    active: BTreeMap<String, Binding>,
    summary_hook: Option<Arc<dyn CodexSummaryHook>>,
    compaction_notices: std::collections::BTreeSet<String>,
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
            compaction_notices: std::collections::BTreeSet::new(),
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
                    .is_some_and(|binding| !binding.idle())
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
                match self.runner.refresh_codex_mcp_config(&prompt.run_id.0) {
                    Ok(true) => {
                        self.active.remove(&prompt.run_id.0);
                    }
                    Ok(false) => {}
                    Err(error) => return self.fail_dispatch(&prompt, host_epoch, error.into()),
                }
                if let Err(error) = self.release_outdated_session(&prompt.run_id.0) {
                    return self.fail_dispatch(&prompt, host_epoch, error);
                }
                let active_run = self.active.get(&prompt.run_id.0);
                let reuses_settled_session = active_run.is_some_and(|binding| binding.settled)
                    && self.runner.has_codex_session(&prompt.run_id.0);
                let other_settled_runs = self
                    .active
                    .iter()
                    .filter(|(run_id, binding)| {
                        run_id.as_str() != prompt.run_id.0.as_str()
                            && binding.idle()
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
                    (*prompt).clone(),
                    host_epoch,
                )
                .or_else(|error| self.fail_dispatch(&prompt, host_epoch, error))
            }
        }
    }
    pub(crate) fn has_settled_session(&self) -> bool {
        self.active
            .iter()
            .any(|(run_id, binding)| binding.idle() && self.runner.has_codex_session(run_id))
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
