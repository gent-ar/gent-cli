use gent_ports::{ClaurstCheckpoint, Ledger, RunCheckpointLedger};
use gent_runtime::RuntimeError;
use gent_types::RunCheckpointRecord;

use super::PrivateClaurstIngress;
use super::validation::{checkpoint_id, event_id, invariant};

impl<L, B> PrivateClaurstIngress<L, B>
where
    L: Clone
        + std::fmt::Debug
        + Ledger
        + gent_ports::GoalLedger
        + RunCheckpointLedger
        + gent_ports::RunLifecycleFactLedger
        + gent_ports::NormalizedSessionBatchLedger
        + gent_ports::PendingPermissionLedger
        + gent_ports::PolicyLedger
        + gent_ports::AgentChatWorkspaceLedger,
    B: gent_ports::PrivateClaurstBridge,
{
    pub(super) fn save_checkpoint(
        &self,
        binding: &gent_ports::ClaurstSessionBinding,
        checkpoint: ClaurstCheckpoint,
        terminal_kind: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let sequence = u64::try_from(self.coordinator.run_checkpoints(&binding.run_id)?.len())
            .expect("checkpoint count fits u64")
            + 1;
        let kind =
            terminal_kind.map_or_else(|| format!("fact-{}", checkpoint.cursor), str::to_owned);
        let event_cursor = self
            .ledger
            .find_event(&event_id(&binding.source_id, &kind))?
            .map_or(0, |event| event.cursor);
        if event_cursor == 0 {
            return Err(invariant(
                "private Claurst checkpoint has no durable source event",
            ));
        }
        self.coordinator.save_run_checkpoint(&RunCheckpointRecord {
            checkpoint_id: checkpoint_id(
                &binding.source_id,
                checkpoint.cursor,
                terminal_kind.is_some(),
            ),
            run_id: binding.run_id.clone(),
            sequence,
            event_cursor,
            state_digest_sha256: checkpoint.state_digest_sha256,
        })
    }
}
