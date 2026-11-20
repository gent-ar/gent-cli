use gent_ports::PrivateClaurstBridge;
use gent_types::{AgentChatPromptSaved, DurableTurnPhase};

use super::compaction::Work;
use super::{ClaurstLifecycleLedger, ClaurstPromptLifecycle};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    pub(super) fn warm(&self, conversation_id: &str, run_id: &str) {
        let busy = self.busy_conversations();
        if !self.recovery_complete
            || self.shutdown_requested
            || busy.iter().any(|busy| busy == conversation_id)
        {
            return;
        }
        let runtime = std::sync::Arc::clone(&self.runtime);
        let (conversation_id, run_id) = (conversation_id.to_owned(), run_id.to_owned());
        tokio::spawn(async move {
            if let Err(error) = runtime.warm(&conversation_id, &run_id, &busy).await {
                eprintln!("Claurst warm-up for run {run_id} was unavailable: {error}");
            }
        });
    }

    pub(super) fn reap(&self, now: std::time::Instant) {
        let busy = self.busy_conversations();
        let runtime = std::sync::Arc::clone(&self.runtime);
        tokio::spawn(async move {
            if let Err(error) = runtime.reap_idle(now, &busy).await {
                eprintln!("Claurst idle runtime was not stopped: {error}");
            }
        });
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
        let running = self.active.iter().find_map(|(source_id, active)| {
            (active.saved.run_id.0 == run_id
                && turn_id.is_none_or(|turn_id| active.saved.message.turn_id == turn_id))
            .then(|| (source_id.clone(), matches!(active.work, Work::Launching(_))))
        });
        match running {
            Some((source_id, true)) => {
                self.active.remove(&source_id);
            }
            Some(_) if self.interrupt_compaction(run_id) => return Ok(()),
            Some(_) => return self.ingress.cancel_run(run_id).await.map_err(display),
            None => {}
        }
        let unstarted = self
            .ledger
            .interrupt_unstarted_agent_chat_prompt(run_id, turn_id, self.host_epoch)
            .map_err(display)?;
        (self.interrupt_background(run_id).await? || unstarted)
            .then_some(())
            .ok_or_else(|| "the interrupt target turn is no longer active".into())
    }

    pub(super) async fn drain_active(&mut self) -> Result<bool, String> {
        let sources: Vec<_> = self.active.keys().cloned().collect();
        let mut progressed = false;
        for source_id in sources {
            match self.active.get(&source_id).map(|active| &active.work) {
                Some(Work::Compacting(_)) => {
                    progressed |= self.drain_compaction(&source_id).await?;
                    continue;
                }
                Some(Work::Launching(_)) => {
                    progressed |= self.drain_launch(&source_id).await?;
                    continue;
                }
                _ => {}
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
                        &format!(
                            "{}claurst-drain-failed",
                            gent_types::AgentChatFailureCode::ProviderFailed.event_prefix()
                        ),
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
                    progressed = true;
                    continue;
                }
            };
            progressed |= drained.progressed;
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
                    self.follow_background(source_id, saved).await;
                } else {
                    self.runtime
                        .after_prompt_failed(
                            &saved.message.conversation_id,
                            if terminal_phase == DurableTurnPhase::Interrupted {
                                gent_types::ToolPhase::Interrupted
                            } else {
                                gent_types::ToolPhase::Failed
                            },
                        )
                        .await?;
                }
            }
        }
        Ok(progressed)
    }

    pub(super) async fn stop_failed_runtime(
        &self,
        saved: &AgentChatPromptSaved,
        error: String,
    ) -> String {
        match self
            .runtime
            .after_prompt_failed(
                &saved.message.conversation_id,
                gent_types::ToolPhase::Failed,
            )
            .await
        {
            Ok(()) => error,
            Err(cleanup) => format!("{error}; Claurst runtime cleanup failed: {cleanup}"),
        }
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

pub(super) fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
