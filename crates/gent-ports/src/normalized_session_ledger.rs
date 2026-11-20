use gent_types::{
    DurableTurnPhase, HostEpoch, NormalizedSessionBatch, NormalizedSessionBatchResult,
};

use crate::LedgerError;

pub trait NormalizedSessionBatchLedger: Send + Sync {
    fn append_normalized_session_batches(
        &self,
        batches: &[NormalizedSessionBatch],
    ) -> Result<Vec<NormalizedSessionBatchResult>, LedgerError>;

    fn append_normalized_session_batch(
        &self,
        batch: &NormalizedSessionBatch,
    ) -> Result<NormalizedSessionBatchResult, LedgerError> {
        self.append_normalized_session_batches(std::slice::from_ref(batch))?
            .pop()
            .ok_or_else(|| {
                LedgerError::Invariant("normalized session batch was not recorded".into())
            })
    }

    fn start_provider_turn(
        &self,
        conversation_id: &str,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<String, LedgerError>;

    fn settle_provider_turn(
        &self,
        turn_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), LedgerError>;
}
