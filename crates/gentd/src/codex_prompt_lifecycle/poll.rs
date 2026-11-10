use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_ports::PublicProviderRunError;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{DurableTurnPhase, HostEpoch};

use super::{CodexPromptLifecycle, CodexPromptPoll, phase};

impl<L, D, R> CodexPromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: super::CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
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
        let promoted = self.promote_queued_permission(run_id, host_epoch)?;
        let Some(effects) = effects.or_else(|| promoted.map(|_| Vec::new())) else {
            return Ok(None);
        };
        let mut facts: u16 = promoted.unwrap_or(0);
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
                        self.admit_permission_request(run_id, host_epoch, request)?,
                    );
                }
                CodexRunnerEffect::Steer(outcome) => {
                    if terminal.is_some()
                        && !matches!(
                            &outcome,
                            gent_drivers::codex_session::CodexSteerOutcome::Rejected { .. }
                        )
                    {
                        self.retire_targeted_steers(run_id, host_epoch)?;
                    }
                    self.resolve_steer(run_id, host_epoch, outcome)?;
                    facts = facts.saturating_add(1);
                }
                CodexRunnerEffect::ResumeUnavailable => {
                    if self.recover_upgraded_session(run_id, host_epoch)? {
                        return Ok(Some(CodexPromptPoll {
                            facts: facts.saturating_add(1),
                            exited: false,
                        }));
                    }
                    self.record_wire(run_id, host_epoch, &session_unavailable())?;
                    terminal = Some(DurableTurnPhase::Failed);
                    facts = facts.saturating_add(1);
                }
                CodexRunnerEffect::Exited { code } => {
                    if let Some(message_id) = self
                        .active
                        .get(run_id)
                        .filter(|binding| !binding.settled && !binding.releasing)
                        .map(|binding| binding.prompt.message.message_id.clone())
                    {
                        let error = crate::public_driver_runtime::run_failure::provider_exited(
                            "Codex", code,
                        );
                        self.runtime.fail_claimed_prompt(
                            &message_id,
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
                    self.settle_if_open(run_id, host_epoch, DurableTurnPhase::Failed)?;
                    self.release_steers(run_id, host_epoch)?;
                    self.active.remove(run_id);
                    return Ok(Some(CodexPromptPoll {
                        facts,
                        exited: true,
                    }));
                }
            }
        }
        if let Some(phase) = terminal {
            self.retire_targeted_steers(run_id, host_epoch)?;
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
}

fn session_unavailable() -> gent_drivers::public_protocol::PublicWireFact {
    gent_drivers::public_protocol::PublicWireFact::Event(
        gent_types::NormalizedProviderEvent::ProviderFailure {
            classification: gent_types::ProviderFailureClassification::SessionUnavailable,
            message: gent_types::PROVIDER_SESSION_UNAVAILABLE_NOTICE.into(),
        },
    )
}
