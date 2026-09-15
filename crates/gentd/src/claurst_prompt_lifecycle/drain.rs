use gent_ports::PrivateClaurstBridge;
use gent_types::DurableTurnPhase;

use super::{ClaurstLifecycleLedger, ClaurstPromptLifecycle, display};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
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
}
