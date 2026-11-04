use std::{collections::BTreeMap, sync::Arc};

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::{AgentChatPromptDispatchResult, RuntimeError};
use gent_types::{AgentChatPromptSaved, DurableTurnPhase, HostEpoch};

use crate::public_driver_runtime::{PublicDriverFact, PublicDriversRuntime};

mod containment;
mod execution;
mod launch;
mod permission;
mod poll;
mod prewarm;
mod recovery;
mod scheduler;
mod start;
mod steer;
mod summary;
mod terminal;
mod types;
mod warm_pool;
#[allow(unused_imports)]
pub(crate) use execution::{ClaudePromptExecution, ClaudePromptRunner, ClaudePromptStart};
pub(crate) use scheduler::ClaudeLifecycleTick;
pub(crate) use summary::ClaudeSummaryHook;
pub(crate) use types::{ClaudePromptDispatchOutcome, ClaudePromptPoll};

#[derive(Clone, Debug)]
pub(super) struct Binding {
    prompt: AgentChatPromptSaved,
    sequence: u64,
    settled: bool,
    interrupt_requested: bool,
    steers: Vec<AgentChatPromptSaved>,
    session_recovery: SessionRecovery,
    failure_reported_turn: Option<String>,
    open_children: std::collections::BTreeMap<String, bool>,
    last_dispatch: u64,
    idle_since: Option<std::time::Instant>,
    awaits_first_input: bool,
}

impl Binding {
    fn idle(&self) -> bool {
        self.settled
            && !self.interrupt_requested
            && self.steers.is_empty()
            && self.open_children.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SessionRecovery {
    NotNeeded,
    Unavailable,
    Recovered,
}

#[derive(Debug)]
pub(crate) struct ClaudePromptLifecycle<L, D, R> {
    runtime: PublicDriversRuntime<L, D, R>,
    runner: D,
    coordinator_id: String,
    active: BTreeMap<String, Binding>,
    warm: BTreeMap<String, prewarm::WarmBinding>,
    summary_hook: Option<Arc<dyn ClaudeSummaryHook>>,
    compaction_notices: std::collections::BTreeSet<String>,
    shutdown_signaled: bool,
    warm_capacity: usize,
    dispatches: u64,
}

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: ClaudePromptExecution + Clone,
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
            warm: BTreeMap::new(),
            summary_hook: None,
            compaction_notices: std::collections::BTreeSet::new(),
            shutdown_signaled: false,
            warm_capacity: 1,
            dispatches: 0,
        }
    }

    pub(crate) fn with_warm_capacity(mut self, capacity: usize) -> Self {
        self.warm_capacity = capacity.max(1);
        self
    }

    pub(crate) fn with_summary_hook(mut self, hook: Arc<dyn ClaudeSummaryHook>) -> Self {
        self.summary_hook = Some(hook);
        self
    }

    pub(crate) fn dispatch_next(
        &mut self,
        host_epoch: HostEpoch,
    ) -> Result<ClaudePromptDispatchOutcome, RuntimeError> {
        let excluded_run_ids = self
            .active
            .keys()
            .filter(|run_id| {
                self.active
                    .get(*run_id)
                    .is_some_and(|binding| !binding.settled || binding.interrupt_requested)
            })
            .cloned()
            .map(gent_types::AgentChatRunId)
            .collect::<Vec<_>>();
        match self.runtime.claim_prompt_excluding_runs(
            &self.coordinator_id,
            host_epoch,
            gent_types::AgentChatProvider::Claude,
            &excluded_run_ids,
        )? {
            AgentChatPromptDispatchResult::DeniedObserver => {
                Ok(ClaudePromptDispatchOutcome::Denied)
            }
            AgentChatPromptDispatchResult::Empty => Ok(ClaudePromptDispatchOutcome::Empty),
            AgentChatPromptDispatchResult::Claimed(prompt) => {
                let held = match self.adopt_warm(&prompt) {
                    Ok(held) => held,
                    Err(error) => return self.fail_dispatch(&prompt, host_epoch, error),
                };
                if let Err(error) = self.release_outdated_session(&prompt.run_id.0) {
                    return self.fail_dispatch(&prompt, host_epoch, error);
                }
                let reuses_session = self.active.contains_key(&prompt.run_id.0)
                    && self.runner.has_claude_session(&prompt.run_id.0);
                if !reuses_session {
                    self.make_room_for(&prompt)?;
                }
                let outcome = start::prompt(
                    &self.runtime,
                    &self.runner,
                    &self.coordinator_id,
                    &mut self.active,
                    (*prompt).clone(),
                    host_epoch,
                )
                .or_else(|error| self.fail_dispatch(&prompt, host_epoch, error));
                self.mark_dispatched(&prompt.run_id.0);
                self.flush_held(&prompt.run_id.0, host_epoch, held)?;
                outcome
            }
        }
    }

    pub(crate) fn interrupt(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        if !self.active.contains_key(run_id) {
            return Err(missing_binding());
        }
        self.runner
            .signal_claude_process(
                run_id,
                gent_drivers::interrupt::ProcessTreeSignal::Interrupt,
            )
            .map_err(RuntimeError::from)?;
        self.active
            .get_mut(run_id)
            .ok_or_else(missing_binding)?
            .interrupt_requested = true;
        Ok(())
    }

    pub(crate) fn interrupt_turn(
        &mut self,
        run_id: &str,
        turn_id: &str,
    ) -> Result<(), RuntimeError> {
        // The lifecycle owner is locked through validation and provider signal;
        // a completed/replaced turn cannot consume a delayed Stop.
        let binding = self.active.get(run_id).ok_or_else(missing_binding)?;
        if binding.settled || binding.prompt.message.turn_id != turn_id {
            return Err(missing_binding());
        }
        self.interrupt(run_id)
    }

    fn record_exit(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        code: Option<i32>,
    ) -> Result<(), RuntimeError> {
        self.record_terminal(
            run_id,
            host_epoch,
            &format!(
                "providerExited:{}",
                code.map_or_else(|| "unknown".into(), |value| value.to_string())
            ),
        )
    }

    fn record_terminal(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        let event_id = self.next_event_id(run_id, host_epoch, "terminal")?;
        self.runtime.record(
            run_id,
            &self.coordinator_id,
            host_epoch,
            PublicDriverFact::SessionEffect {
                event_id,
                effect: gent_drivers::SessionEffect::Terminal {
                    reason: reason.into(),
                },
            },
        )?;
        Ok(())
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
            if phase == DurableTurnPhase::Completed {
                if let Some(hook) = &self.summary_hook {
                    let _ = hook.schedule(&binding.prompt.message.conversation_id);
                }
            }
        }
        Ok(())
    }

    fn next_event_id(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        kind: &str,
    ) -> Result<String, RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        binding.sequence = binding.sequence.saturating_add(1);
        Ok(format!(
            "claude:{}:{run_id}:{}:{kind}:{}",
            host_epoch.0, binding.prompt.message.turn_id, binding.sequence
        ))
    }
}

fn missing_binding() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "Claude runner has no durable prompt binding".into(),
    ))
}

mod record;
