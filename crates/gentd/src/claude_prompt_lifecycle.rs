use std::{collections::BTreeMap, sync::Arc};

use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, PublicProviderRunError, TranscriptLedger,
};
use gent_runtime::{AgentChatPromptDispatchResult, RuntimeError};
use gent_types::{AgentChatPromptSaved, DurableTurnPhase, HostEpoch};

use crate::public_driver_runtime::{NormalizedSessionFact, PublicDriverFact, PublicDriversRuntime};

mod execution;
mod permission;
mod scheduler;
mod start;
mod summary;
mod terminal;
mod types;
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
}

#[derive(Debug)]
pub(crate) struct ClaudePromptLifecycle<L, D, R> {
    runtime: PublicDriversRuntime<L, D, R>,
    runner: D,
    coordinator_id: String,
    active: BTreeMap<String, Binding>,
    summary_hook: Option<Arc<dyn ClaudeSummaryHook>>,
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
            summary_hook: None,
        }
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
                    .is_some_and(|binding| !binding.settled)
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
                // Retain the claimed conversation's live session.  Other idle
                // sessions are explicitly released before a different run can
                // consume the bounded process capacity.
                let other_settled = self
                    .active
                    .iter()
                    .filter(|(run_id, binding)| {
                        run_id.as_str() != prompt.run_id.0.as_str()
                            && binding.settled
                            && self.runner.has_claude_session(run_id)
                    })
                    .map(|(run_id, _)| run_id.clone())
                    .collect::<Vec<_>>();
                for run_id in other_settled {
                    self.runner.release_claude_session(&run_id)?;
                    self.active.remove(&run_id);
                }
                start::prompt(
                    &self.runtime,
                    &self.runner,
                    &self.coordinator_id,
                    &mut self.active,
                    *prompt,
                    host_epoch,
                )
            }
        }
    }

    pub(crate) fn interrupt(&self, run_id: &str) -> Result<(), RuntimeError> {
        if !self.active.contains_key(run_id) {
            return Err(missing_binding());
        }
        self.runner
            .signal_claude_process(
                run_id,
                gent_drivers::interrupt::ProcessTreeSignal::Interrupt,
            )
            .map_err(RuntimeError::from)
    }

    pub(crate) fn poll(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<ClaudePromptPoll>, RuntimeError> {
        let effects = self.runner.poll_claude_prompt(run_id).map_err(|_| {
            RuntimeError::ProviderRun(PublicProviderRunError::Failed(
                "provider poll unavailable".into(),
            ))
        })?;
        let Some(effects) = effects else {
            return Ok(None);
        };
        if !self.active.contains_key(run_id) {
            return Err(missing_binding());
        }
        let mut facts: u16 = 0;
        let mut terminal = None;
        for effect in effects {
            match effect {
                ClaudeRunnerEffect::Fact(fact) => {
                    terminal = terminal.or_else(|| terminal::phase(&fact));
                    self.record_wire(run_id, host_epoch, &fact)?;
                    facts = facts.saturating_add(1);
                }
                ClaudeRunnerEffect::PermissionRequest(request) => {
                    let permission = self.record_permission_request(run_id, host_epoch, request)?;
                    if permission.terminal {
                        terminal = Some(DurableTurnPhase::Failed);
                    }
                    facts = facts.saturating_add(permission.facts);
                }
                ClaudeRunnerEffect::Exited { code } => {
                    self.record_exit(run_id, host_epoch, code)?;
                    self.settle_if_open(run_id, host_epoch, DurableTurnPhase::Failed)?;
                    self.active.remove(run_id);
                    return Ok(Some(ClaudePromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        if let Some(phase) = terminal {
            self.settle_if_open(run_id, host_epoch, phase)?;
        }
        Ok(Some(ClaudePromptPoll {
            facts,
            exited: false,
        }))
    }

    fn record_wire(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        fact: &PublicWireFact,
    ) -> Result<bool, RuntimeError> {
        if matches!(fact, PublicWireFact::SessionStarted { .. }) {
            let event_id = self.next_event_id(run_id, host_epoch, "session")?;
            self.runtime.record(
                run_id,
                &self.coordinator_id,
                host_epoch,
                PublicDriverFact::PublicWire {
                    event_id,
                    fact: fact.clone(),
                },
            )?;
            return Ok(false);
        }
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let lifecycle_event_id = self.next_event_id(run_id, host_epoch, "wire")?;
        let transcript_event_id = self.next_event_id(run_id, host_epoch, "transcript")?;
        let activity_event_id = self.next_event_id(run_id, host_epoch, "activity")?;
        let input = NormalizedSessionFact {
            run_id: run_id.into(),
            conversation_id: binding.prompt.message.conversation_id,
            turn_id: binding.prompt.message.turn_id,
            host_epoch,
            lifecycle_event_id,
            transcript_event_id,
            activity_event_id,
            fact: fact.clone(),
        };
        let record = self
            .runtime
            .record_normalized_session(&self.coordinator_id, &input)?;
        let _ = record;
        Ok(false)
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
            "claude:{}:{run_id}:{kind}:{}",
            host_epoch.0, binding.sequence
        ))
    }
}

fn missing_binding() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "Claude runner has no durable prompt binding".into(),
    ))
}
