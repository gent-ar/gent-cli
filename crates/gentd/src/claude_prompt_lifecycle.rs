//! Dormant daemon-owned Claude prompt lifecycle over durable dispatch and normalized facts.

use std::collections::BTreeMap;

use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    RunProjectionLedger, TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchResult, AgentChatTranscriptAppendRequest, ProviderActivityFact,
    RuntimeError,
};
use gent_types::{
    AgentChatPromptSaved, HostEpoch, NormalizedProviderEvent, NormalizedTranscriptKind,
};

use crate::public_driver_runtime::{PublicDriverFact, PublicDriversRuntime};

mod activity;
mod execution;
mod scheduler;
mod start;
#[allow(unused_imports)]
pub(crate) use execution::{ClaudePromptExecution, ClaudePromptRunner, ClaudePromptStart};
pub(crate) use scheduler::ClaudeLifecycleTick;

/// Outcome of claiming and attempting one durable Claude prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaudePromptDispatchOutcome {
    Denied,
    Busy,
    Empty,
    Started { run_id: String },
    Unprovable { run_id: String },
}

/// Bounded result from draining one daemon-owned Claude process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClaudePromptPoll {
    pub facts: u16,
    pub exited: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Binding {
    prompt: AgentChatPromptSaved,
    sequence: u64,
    settled: bool,
}

/// A dormant host that cannot be built from the shipped observer runtime.
#[derive(Debug)]
pub(crate) struct ClaudePromptLifecycle<L, D, R> {
    runtime: PublicDriversRuntime<L, D, R>,
    runner: D,
    coordinator_id: String,
    active: BTreeMap<String, Binding>,
}

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
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
        }
    }

    /// Claims one Claude prompt. Every turn uses a new one-shot process, optionally resumed.
    pub(crate) fn dispatch_next(
        &mut self,
        host_epoch: HostEpoch,
    ) -> Result<ClaudePromptDispatchOutcome, RuntimeError> {
        match self.runtime.claim_prompt(
            &self.coordinator_id,
            host_epoch,
            gent_types::AgentChatProvider::Claude,
        )? {
            AgentChatPromptDispatchResult::DeniedObserver => {
                Ok(ClaudePromptDispatchOutcome::Denied)
            }
            AgentChatPromptDispatchResult::Empty => Ok(ClaudePromptDispatchOutcome::Empty),
            AgentChatPromptDispatchResult::Claimed(prompt) => {
                if self.active.contains_key(&prompt.run_id.0) {
                    self.runtime.release_prompt_claim(
                        &prompt.message.message_id,
                        &self.coordinator_id,
                        host_epoch,
                    )?;
                    return Ok(ClaudePromptDispatchOutcome::Busy);
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

    /// Persists facts before projections. A ready result settles its prompt but retains ownership
    /// until stream EOF and process exit prove the one-shot process is drained.
    pub(crate) fn poll(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<ClaudePromptPoll>, RuntimeError> {
        let Ok(effects) = self.runner.poll_claude_prompt(run_id) else {
            return self.settle_poll_failure(run_id, host_epoch);
        };
        let Some(effects) = effects else {
            return Ok(None);
        };
        if !self.active.contains_key(run_id) {
            return Err(missing_binding());
        }
        let mut facts: u16 = 0;
        let mut terminal = false;
        for effect in effects {
            match effect {
                ClaudeRunnerEffect::Fact(fact) => {
                    terminal |= self.record_wire(run_id, host_epoch, &fact)?;
                    facts = facts.saturating_add(1);
                }
                ClaudeRunnerEffect::Exited { code } => {
                    self.record_exit(run_id, host_epoch, code)?;
                    self.settle_if_open(run_id, host_epoch)?;
                    self.active.remove(run_id);
                    return Ok(Some(ClaudePromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        if terminal {
            self.settle_if_open(run_id, host_epoch)?;
        }
        Ok(Some(ClaudePromptPoll {
            facts,
            exited: false,
        }))
    }

    fn settle_poll_failure(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<ClaudePromptPoll>, RuntimeError> {
        self.record_terminal(run_id, host_epoch, "providerPollFailure")?;
        self.settle_if_open(run_id, host_epoch)?;
        self.active.remove(run_id);
        Ok(Some(ClaudePromptPoll {
            facts: 0,
            exited: true,
        }))
    }

    fn record_wire(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        fact: &PublicWireFact,
    ) -> Result<bool, RuntimeError> {
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let event_id = self.next_event_id(run_id, "wire")?;
        self.runtime.record(
            run_id.into(),
            &self.coordinator_id,
            host_epoch,
            PublicDriverFact::PublicWire {
                event_id,
                fact: fact.clone(),
            },
        )?;
        if let PublicWireFact::Event(NormalizedProviderEvent::Output { text, is_partial }) = fact {
            let event_id = self.next_event_id(run_id, "transcript")?;
            self.runtime.record(
                run_id.into(),
                &self.coordinator_id,
                host_epoch,
                PublicDriverFact::Transcript(AgentChatTranscriptAppendRequest {
                    conversation_id: gent_types::AgentChatConversationId(
                        binding.prompt.message.conversation_id.clone(),
                    ),
                    run_id: binding.prompt.run_id.clone(),
                    turn_id: binding.prompt.message.turn_id.clone(),
                    event_id,
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: text.clone(),
                    is_partial: *is_partial,
                }),
            )?;
        }
        if let Some(activity) = activity::fact(&binding, host_epoch, fact) {
            let event_id = self.next_event_id(run_id, "activity")?;
            self.runtime.record(
                run_id.into(),
                &self.coordinator_id,
                host_epoch,
                PublicDriverFact::Activity(ProviderActivityFact { event_id, activity }),
            )?;
        }
        Ok(is_terminal(fact))
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
        let event_id = self.next_event_id(run_id, "terminal")?;
        self.runtime.record(
            run_id.into(),
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
        Ok(format!("claude:{run_id}:{kind}:{}", binding.sequence))
    }
}

fn is_terminal(fact: &PublicWireFact) -> bool {
    matches!(
        fact,
        PublicWireFact::Lifecycle(gent_types::NormalizedLifecycleSignal::RootPhase { phase })
            if matches!(phase, gent_types::TurnPhase::Ready | gent_types::TurnPhase::Interrupted | gent_types::TurnPhase::Failed)
    )
}

fn missing_binding() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "Claude runner has no durable prompt binding".into(),
    ))
}
