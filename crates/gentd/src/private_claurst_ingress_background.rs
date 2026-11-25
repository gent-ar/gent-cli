use gent_ports::{
    ClaurstDrainRequest, ClaurstSourceId, Ledger, MAX_PRIVATE_CLAURST_DRAIN_FACTS,
    NormalizedSessionBatchLedger, PrivateClaurstBridge, RunCheckpointLedger,
    RunLifecycleFactLedger, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use super::PrivateClaurstIngress;
use super::validation::{invariant, validate_batch};

impl<L, B> PrivateClaurstIngress<L, B>
where
    L: Clone
        + std::fmt::Debug
        + Ledger
        + gent_ports::GoalLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + NormalizedSessionBatchLedger
        + TranscriptLedger
        + gent_ports::PendingPermissionLedger
        + gent_ports::PolicyLedger
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::AgentChatConversationConfigLedger
        + gent_ports::AgentChatReadLedger,
    B: PrivateClaurstBridge,
{
    pub(crate) async fn runs_background_commands(&self, source_id: &ClaurstSourceId) -> bool {
        match self.sources.get(source_id) {
            Some(state) => self.bridge.runs_background_commands(&state.binding).await,
            None => false,
        }
    }

    pub(crate) async fn cancel_background_commands(
        &self,
        source_id: &ClaurstSourceId,
    ) -> Result<(), RuntimeError> {
        let state = self
            .sources
            .get(source_id)
            .ok_or_else(|| invariant("private Claurst source is not bound"))?;
        self.bridge.cancel(state.binding.clone()).await?;
        Ok(())
    }

    pub(crate) async fn drain_background_commands(
        &mut self,
        source_id: &ClaurstSourceId,
        host_epoch: HostEpoch,
    ) -> Result<bool, RuntimeError> {
        let state = self
            .sources
            .get(source_id)
            .cloned()
            .filter(|state| state.terminal)
            .ok_or_else(|| invariant("private Claurst source has no ended turn"))?;
        let request = ClaurstDrainRequest {
            run_id: state.binding.run_id.clone(),
            source_id: source_id.clone(),
            after_cursor: state.after_cursor,
            limit: MAX_PRIVATE_CLAURST_DRAIN_FACTS,
        };
        let batch = self.bridge.drain(request.clone()).await?;
        let checkpoint = validate_batch(&request, &state.binding, &batch)?;
        if batch.terminal.is_some() {
            return Err(invariant(
                "private Claurst background commands reopened an ended turn",
            ));
        }
        self.record_facts(&state, &batch.facts, host_epoch)?;
        self.admit_permissions(&state, &batch.permissions, host_epoch)
            .await?;
        self.sources
            .get_mut(source_id)
            .expect("bound source remains present")
            .after_cursor = checkpoint.cursor;
        Ok(!batch.facts.is_empty() || !batch.permissions.is_empty())
    }
}
