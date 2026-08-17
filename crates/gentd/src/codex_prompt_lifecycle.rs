//! Dormant daemon-owned Codex prompt lifecycle over durable dispatch and normalized facts.

use std::collections::BTreeMap;

use gent_drivers::codex_runner::CodexRunnerEffect;
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
}

/// A dormant host that cannot be built from the shipped observer runtime.
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
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
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
                if self.active.contains_key(&prompt.run_id.0) {
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

    /// Persists each normalized fact before it becomes a transcript or activity update.
    pub(crate) fn poll(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<CodexPromptPoll>, RuntimeError> {
        let Ok(effects) = self.runner.poll_codex_prompt(run_id) else {
            return self.settle_poll_failure(run_id, host_epoch);
        };
        let Some(effects) = effects else {
            return Ok(None);
        };
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let mut facts = 0;
        for effect in effects {
            match effect {
                CodexRunnerEffect::Fact(fact) => {
                    let terminal = self.record_wire(run_id, host_epoch, &binding, &fact)?;
                    facts += 1;
                    if terminal {
                        self.runtime.settle_prompt(
                            &binding.prompt.message.message_id,
                            &self.coordinator_id,
                            host_epoch,
                        )?;
                        self.active.remove(run_id);
                        return Ok(Some(CodexPromptPoll {
                            facts,
                            exited: false,
                        }));
                    }
                }
                CodexRunnerEffect::Exited { code } => {
                    let event_id = self.next_event_id(run_id, "exit")?;
                    self.runtime.record(
                        run_id.into(),
                        &self.coordinator_id,
                        host_epoch,
                        PublicDriverFact::SessionEffect {
                            event_id,
                            effect: gent_drivers::SessionEffect::Terminal {
                                reason: format!(
                                    "providerExited:{}",
                                    code.map_or_else(
                                        || "unknown".into(),
                                        |value| value.to_string()
                                    )
                                ),
                            },
                        },
                    )?;
                    self.runtime.settle_prompt(
                        &binding.prompt.message.message_id,
                        &self.coordinator_id,
                        host_epoch,
                    )?;
                    self.active.remove(run_id);
                    return Ok(Some(CodexPromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        Ok(Some(CodexPromptPoll {
            facts,
            exited: false,
        }))
    }

    fn settle_poll_failure(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<CodexPromptPoll>, RuntimeError> {
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let event_id = self.next_event_id(run_id, "poll")?;
        self.runtime.record(
            run_id.into(),
            &self.coordinator_id,
            host_epoch,
            PublicDriverFact::SessionEffect {
                event_id,
                // Runner error details can contain provider-local paths or frames. The durable
                // public lifecycle records only this stable, normalized classification.
                effect: gent_drivers::SessionEffect::Terminal {
                    reason: "providerPollFailure".into(),
                },
            },
        )?;
        self.runtime.settle_prompt(
            &binding.prompt.message.message_id,
            &self.coordinator_id,
            host_epoch,
        )?;
        self.active.remove(run_id);
        Ok(Some(CodexPromptPoll {
            facts: 0,
            exited: true,
        }))
    }

    fn record_wire(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        binding: &Binding,
        fact: &PublicWireFact,
    ) -> Result<bool, RuntimeError> {
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
        if let PublicWireFact::Event(NormalizedProviderEvent::Output { text }) = &fact {
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
                    is_partial: true,
                }),
            )?;
        }
        if let Some(activity) = activity::fact(binding, host_epoch, fact) {
            let event_id = self.next_event_id(run_id, "activity")?;
            self.runtime.record(
                run_id.into(),
                &self.coordinator_id,
                host_epoch,
                PublicDriverFact::Activity(ProviderActivityFact { event_id, activity }),
            )?;
        }
        Ok(matches!(
            fact,
            PublicWireFact::Lifecycle(gent_types::NormalizedLifecycleSignal::RootPhase { phase })
                if matches!(
                    phase,
                    gent_types::TurnPhase::Ready
                        | gent_types::TurnPhase::Interrupted
                        | gent_types::TurnPhase::Failed
                )
        ))
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
