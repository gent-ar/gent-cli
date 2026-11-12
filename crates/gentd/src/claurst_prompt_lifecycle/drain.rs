use gent_ports::PrivateClaurstBridge;
use gent_types::{AgentChatPromptSaved, DurableTurnPhase};

use super::{ClaurstLifecycleLedger, ClaurstPromptLifecycle, display};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    pub(super) async fn warm(&self, conversation_id: &str, run_id: &str) -> Result<(), String> {
        let busy = self.busy_conversations();
        if !self.recovery_complete
            || self.shutdown_requested
            || busy.iter().any(|busy| busy == conversation_id)
        {
            return Ok(());
        }
        self.runtime.warm(conversation_id, run_id, &busy).await
    }

    pub(super) fn busy_conversations(&self) -> Vec<String> {
        self.active
            .values()
            .map(|active| active.saved.message.conversation_id.clone())
            .collect()
    }

    pub(super) async fn interrupt(
        &mut self,
        run_id: &str,
        turn_id: Option<&str>,
    ) -> Result<(), String> {
        let running = self.active.values().any(|active| {
            active.saved.run_id.0 == run_id
                && turn_id.is_none_or(|turn_id| active.saved.message.turn_id == turn_id)
        });
        if !running {
            return self
                .ledger
                .interrupt_unstarted_agent_chat_prompt(run_id, turn_id, self.host_epoch)
                .map_err(display)?
                .then_some(())
                .ok_or_else(|| "the interrupt target turn is no longer active".into());
        }
        if self.interrupt_compaction(run_id) {
            return Ok(());
        }
        self.ingress.cancel_run(run_id).await.map_err(display)
    }

    pub(super) async fn drain_active(&mut self) -> Result<bool, String> {
        let sources: Vec<_> = self.active.keys().cloned().collect();
        let mut active = false;
        for source_id in sources {
            if self
                .active
                .get(&source_id)
                .is_some_and(|active| matches!(active.work, super::compaction::Work::Compacting(_)))
            {
                active |= self.drain_compaction(&source_id).await?;
                continue;
            }
            let drained = match self.ingress.drain(&source_id, self.host_epoch).await {
                Ok(drained) => drained,
                Err(error) => {
                    let saved = self
                        .active
                        .remove(&source_id)
                        .expect("active source remains present while it is drained")
                        .saved;
                    let error = display(error);
                    let failure = self.stop_failed_runtime(&saved, error.clone()).await;
                    self.record_notice(
                        &saved,
                        "claurst-drain-failed",
                        format!("Claurst stopped: {error}"),
                    )?;
                    self.dispatches
                        .settle_terminal(
                            &saved.message.message_id,
                            &self.coordinator_id,
                            self.host_epoch,
                            DurableTurnPhase::Failed,
                        )
                        .map_err(display)?;
                    eprintln!("Claurst run {} stopped: {failure}", saved.run_id.0);
                    continue;
                }
            };
            if drained.terminal {
                let saved = self
                    .active
                    .remove(&source_id)
                    .expect("active source remains present")
                    .saved;
                let terminal_phase = drained
                    .terminal_phase
                    .expect("terminal Claurst drain has a terminal phase");
                self.dispatches
                    .settle_terminal(
                        &saved.message.message_id,
                        &self.coordinator_id,
                        self.host_epoch,
                        terminal_phase,
                    )
                    .map_err(display)?;
                if terminal_phase == DurableTurnPhase::Completed {
                    self.runtime
                        .after_prompt_settled(&saved.message.conversation_id)
                        .await?;
                } else {
                    self.runtime
                        .after_prompt_failed(&saved.message.conversation_id)
                        .await?;
                }
            } else {
                active = true;
            }
        }
        Ok(active)
    }

    pub(super) fn release_claim(&self, saved: &AgentChatPromptSaved) -> Result<(), String> {
        self.dispatches
            .release_claim(
                &saved.message.message_id,
                &self.coordinator_id,
                self.host_epoch,
            )
            .map_err(display)
    }

    pub(super) fn fail_prelaunch(
        &self,
        saved: &AgentChatPromptSaved,
        error: &str,
    ) -> Result<bool, String> {
        self.fail_prelaunch_with(saved, error, None)
    }

    pub(super) fn fail_prelaunch_with(
        &self,
        saved: &AgentChatPromptSaved,
        error: &str,
        code: Option<gent_types::AgentChatFailureCode>,
    ) -> Result<bool, String> {
        self.dispatches
            .fail_prelaunch(
                &saved.message.message_id,
                &self.coordinator_id,
                self.host_epoch,
                error,
                code,
            )
            .map_err(display)?;
        Ok(true)
    }
}
