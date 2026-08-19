//! Dormant daemon-owned Codex prompt lifecycle over durable dispatch and normalized facts.

use std::collections::BTreeMap;

use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PublicProviderResolver, PublicProviderRunError, TranscriptLedger,
};
use gent_runtime::{AgentChatPromptDispatchResult, RuntimeError};
use gent_types::{AgentChatPromptSaved, HostEpoch};

use crate::public_driver_runtime::{NormalizedSessionFact, PublicDriverFact, PublicDriversRuntime};

mod execution;
mod scheduler;
mod start;
pub(crate) use execution::CodexPromptExecution;

/// Outcome of claiming and attempting one durable Codex prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexPromptDispatchOutcome {
    Denied,
    Busy,
    Empty,
    Started { run_id: String },
    Unprovable { run_id: String },
}
/// Bounded result from draining one daemon-owned Codex process.
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
}

#[derive(Debug)]
pub(crate) struct CodexPromptLifecycle<L, D, R> {
    runtime: PublicDriversRuntime<L, D, R>,
    runner: D,
    coordinator_id: String,
    working_directory: Option<String>,
    active: BTreeMap<String, Binding>,
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
        + gent_ports::ConversationContentReader,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    /// Binds a single coordinator to the same runner clone held by public-run reservation.
    #[must_use]
    pub(crate) fn new(
        runtime: PublicDriversRuntime<L, D, R>,
        coordinator_id: String,
        working_directory: Option<String>,
    ) -> Self {
        let runner = runtime.runner();
        Self {
            runtime,
            runner,
            coordinator_id,
            working_directory,
            active: BTreeMap::new(),
        }
    }

    /// Claims and starts one Codex-only prompt after durable launch ambiguity is recorded.
    pub(crate) fn dispatch_next(
        &mut self,
        host_epoch: HostEpoch,
    ) -> Result<CodexPromptDispatchOutcome, RuntimeError> {
        match self.runtime.claim_prompt(
            &self.coordinator_id,
            host_epoch,
            gent_types::AgentChatProvider::Codex,
        )? {
            AgentChatPromptDispatchResult::DeniedObserver => Ok(CodexPromptDispatchOutcome::Denied),
            AgentChatPromptDispatchResult::Empty => Ok(CodexPromptDispatchOutcome::Empty),
            AgentChatPromptDispatchResult::Claimed(prompt) => {
                let active_run = self.active.get(&prompt.run_id.0);
                let reuses_settled_session = active_run.is_some_and(|binding| binding.settled)
                    && self.runner.has_codex_session(&prompt.run_id.0);
                let another_session_is_owned = self.active.iter().any(|(run_id, binding)| {
                    run_id != &prompt.run_id.0
                        && binding.settled
                        && self.runner.has_codex_session(run_id)
                });
                if (active_run.is_some() && !reuses_settled_session) || another_session_is_owned {
                    self.runtime.release_prompt_claim(
                        &prompt.message.message_id,
                        &self.coordinator_id,
                        host_epoch,
                    )?;
                    return Ok(CodexPromptDispatchOutcome::Busy);
                }
                start::prompt(
                    &self.runtime,
                    &self.runner,
                    &self.coordinator_id,
                    self.working_directory.as_deref(),
                    &mut self.active,
                    *prompt,
                    host_epoch,
                )
            }
        }
    }

    pub(crate) fn has_settled_session(&self) -> bool {
        self.active
            .iter()
            .any(|(run_id, binding)| binding.settled && self.runner.has_codex_session(run_id))
    }

    /// Persists each normalized fact before it becomes a transcript or activity update.
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
        let mut facts = 0;
        let mut terminal = false;
        for effect in effects {
            match effect {
                CodexRunnerEffect::Fact(fact) => {
                    terminal |= self.record_wire(run_id, host_epoch, &fact)?;
                    facts += 1;
                }
                CodexRunnerEffect::Exited { code } => {
                    self.record_exit(run_id, host_epoch, code)?;
                    self.settle_if_open(run_id, host_epoch)?;
                    self.active.remove(run_id);
                    return Ok(Some(CodexPromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        if terminal {
            self.settle_if_open(run_id, host_epoch)?;
        }
        Ok(Some(CodexPromptPoll {
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
            let event_id = self.next_event_id(run_id, "session")?;
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
        let lifecycle_event_id = self.next_event_id(run_id, "wire")?;
        let transcript_event_id = self.next_event_id(run_id, "transcript")?;
        let activity_event_id = self.next_event_id(run_id, "activity")?;
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
        Ok(record.terminal_signal)
    }

    fn record_exit(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        code: Option<i32>,
    ) -> Result<(), RuntimeError> {
        let event_id = self.next_event_id(run_id, "exit")?;
        self.runtime.record(
            run_id,
            &self.coordinator_id,
            host_epoch,
            PublicDriverFact::SessionEffect {
                event_id,
                effect: gent_drivers::SessionEffect::Terminal {
                    reason: format!(
                        "providerExited:{}",
                        code.map_or_else(|| "unknown".into(), |value| value.to_string())
                    ),
                },
            },
        )?;
        Ok(())
    }

    fn settle_if_open(&mut self, run_id: &str, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if !binding.settled {
            self.runtime.settle_prompt(
                &binding.prompt.message.message_id,
                &self.coordinator_id,
                host_epoch,
            )?;
            binding.settled = true;
        }
        Ok(())
    }

    fn next_event_id(&mut self, run_id: &str, kind: &str) -> Result<String, RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        binding.sequence = binding.sequence.saturating_add(1);
        Ok(format!("codex:{}:{kind}:{}", run_id, binding.sequence))
    }
}

fn missing_binding() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "Codex runner has no durable prompt binding".into(),
    ))
}
