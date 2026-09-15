use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::PublicProviderRunError;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{DurableTurnPhase, HostEpoch};

use super::{
    ClaudePromptExecution, ClaudePromptLifecycle, ClaudePromptPoll, missing_binding, terminal,
};

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
                    let Some(fact) = normalize_user_interrupt(
                        fact,
                        self.active
                            .get(run_id)
                            .is_some_and(|binding| binding.interrupt_requested),
                    ) else {
                        continue;
                    };
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
                ClaudeRunnerEffect::ResumeUnavailable => {
                    let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
                    if binding.session_recovery == super::SessionRecovery::NotNeeded {
                        binding.session_recovery = super::SessionRecovery::Unavailable;
                    }
                }
                ClaudeRunnerEffect::SteerConsumed { message_id } => {
                    if let Some(phase) = terminal.take() {
                        self.settle_turn(run_id, host_epoch, phase)?;
                    }
                    self.consume_steer(run_id, host_epoch, &message_id)?;
                    facts = facts.saturating_add(1);
                }
                ClaudeRunnerEffect::Exited { code } => {
                    self.record_exit(run_id, host_epoch, code)?;
                    if self.recover_unavailable_session(run_id, host_epoch)? {
                        return Ok(Some(ClaudePromptPoll {
                            facts: facts.saturating_add(1),
                            exited: false,
                        }));
                    }
                    let phase = if self
                        .active
                        .get(run_id)
                        .is_some_and(|binding| binding.interrupt_requested)
                    {
                        DurableTurnPhase::Interrupted
                    } else {
                        DurableTurnPhase::Failed
                    };
                    self.settle_if_open(run_id, host_epoch, phase)?;
                    self.release_steers(run_id, host_epoch)?;
                    self.active.remove(run_id);
                    return Ok(Some(ClaudePromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        if let Some(phase) = terminal {
            self.settle_turn(run_id, host_epoch, phase)?;
        }
        Ok(Some(ClaudePromptPoll {
            facts,
            exited: false,
        }))
    }
}

fn normalize_user_interrupt(
    fact: PublicWireFact,
    interrupt_requested: bool,
) -> Option<PublicWireFact> {
    if interrupt_requested
        && matches!(
            fact,
            PublicWireFact::Event(gent_types::NormalizedProviderEvent::ProviderFailure { .. })
        )
    {
        return None;
    }
    if interrupt_requested
        && matches!(
            fact,
            PublicWireFact::Lifecycle(gent_types::NormalizedLifecycleSignal::RootPhase {
                phase: gent_types::TurnPhase::Failed
            })
        )
    {
        return Some(PublicWireFact::Lifecycle(
            gent_types::NormalizedLifecycleSignal::RootPhase {
                phase: gent_types::TurnPhase::Interrupted,
            },
        ));
    }
    Some(fact)
}
