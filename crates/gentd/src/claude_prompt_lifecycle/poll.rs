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
        let promoted = self.promote_queued_permission(run_id, host_epoch)?;
        let Some(effects) = effects.or_else(|| promoted.map(|_| Vec::new())) else {
            return Ok(None);
        };
        self.active
            .get_mut(run_id)
            .ok_or_else(missing_binding)?
            .idle_since = None;
        let mut facts: u16 = promoted.map_or(0, |record| record.facts);
        let mut result = None;
        for effect in effects {
            match effect {
                ClaudeRunnerEffect::Fact(fact) => {
                    let provider_failure = matches!(
                        &fact,
                        PublicWireFact::Event(
                            gent_types::NormalizedProviderEvent::ProviderFailure { .. }
                        )
                    );
                    self.record_wire(run_id, host_epoch, &fact)?;
                    self.track_child(run_id, &fact)?;
                    if provider_failure {
                        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
                        binding.failure_reported_turn = Some(binding.turn_id().to_owned());
                    }
                    if let Some(phase) = terminal::phase(&fact) {
                        self.settle_result(run_id, host_epoch, phase, result.take())?;
                    }
                    facts = facts.saturating_add(1);
                }
                ClaudeRunnerEffect::PermissionCancelled { request_id } => {
                    self.permission_cancelled(run_id, &request_id)?;
                }
                ClaudeRunnerEffect::ContinuationStarted => {
                    self.begin_provider_turn(run_id, host_epoch)?;
                }
                ClaudeRunnerEffect::TurnResult {
                    message_ids,
                    continuation,
                } => {
                    result = Some(super::settlement::TurnResult {
                        message_ids,
                        continuation,
                    });
                }
                ClaudeRunnerEffect::PermissionRequest(request) => {
                    facts = facts.saturating_add(
                        self.admit_permission_request(run_id, host_epoch, request)?
                            .facts,
                    );
                }
                ClaudeRunnerEffect::ResumeUnavailable => {
                    let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
                    if binding.session_recovery == super::SessionRecovery::NotNeeded {
                        binding.session_recovery = super::SessionRecovery::Unavailable;
                    }
                }
                ClaudeRunnerEffect::MessageStarted { message_id } => {
                    self.message_started(run_id, host_epoch, &message_id)?;
                    facts = facts.saturating_add(1);
                }
                ClaudeRunnerEffect::MessageCancelled { message_id } => {
                    self.message_cancelled(run_id, host_epoch, &message_id)?;
                    facts = facts.saturating_add(1);
                }
                ClaudeRunnerEffect::SessionState { idle } => {
                    self.session_reported(run_id, host_epoch, idle)?;
                }
                ClaudeRunnerEffect::Exited { code } => {
                    if let Some(binding) = self.active.get(run_id)
                        && binding.turns.turn_open()
                        && !binding.turns.releasing()
                        && binding.failure_reported_turn.as_deref() != Some(binding.turn_id())
                        && binding.session_recovery != super::SessionRecovery::Unavailable
                    {
                        let error = crate::public_driver_runtime::run_failure::provider_exited(
                            "Claude", code,
                        );
                        self.runtime.fail_claimed_prompt(
                            &binding.prompt.message.message_id,
                            &self.coordinator_id,
                            host_epoch,
                            &error,
                        )?;
                        crate::public_driver_runtime::run_failure::tolerate_run_scoped(
                            self.record_wire(
                                run_id,
                                host_epoch,
                                &crate::public_driver_runtime::run_failure::failure_fact(&error),
                            ),
                        )?;
                    }
                    self.record_exit(run_id, host_epoch, code)?;
                    if self.recover_unavailable_session(run_id, host_epoch)? {
                        return Ok(Some(ClaudePromptPoll {
                            facts: facts.saturating_add(1),
                            exited: false,
                        }));
                    }
                    self.settle_if_open(run_id, host_epoch, DurableTurnPhase::Failed)?;
                    self.release_steers(run_id, host_epoch)?;
                    self.active.remove(run_id);
                    return Ok(Some(ClaudePromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        Ok(Some(ClaudePromptPoll {
            facts,
            exited: false,
        }))
    }
}
