//! Coordinator orchestration for the durable conversation → run → turn hierarchy.

use gent_core::{permits_turn_transition, projected_live_status, restore_projection};
use gent_ports::{
    ConversationArtifactLedger, ConversationLedger, Ledger, LedgerError, RunProjectionLedger,
    RunRecord, TurnPhaseUpdate,
};
use gent_types::{
    ConversationArtifact, ConversationArtifactSummary, ConversationListItem, ConversationRecord,
    ConversationRunStatus, ConversationStatus, ConversationTimeline, ConversationTimelineRun,
    DurableTurnPhase, RunLiveStatus, TurnRecord,
};

use crate::{Coordinator, RuntimeError, to_record};

impl<L> Coordinator<L>
where
    L: Ledger + ConversationLedger,
{
    /// Lists content-free conversations for local selection.
    ///
    /// # Errors
    /// Returns an error when durable hierarchy state cannot be read.
    pub fn conversations(&self) -> Result<Vec<ConversationListItem>, RuntimeError> {
        Ok(self.ledger.list_conversations()?)
    }
    /// Atomically creates a conversation and its immutable root run.
    ///
    /// # Errors
    /// Returns an error when the root does not name the conversation or persistence fails.
    pub fn create_conversation_run(
        &self,
        conversation: &ConversationRecord,
        run: &gent_core::Run,
    ) -> Result<(), RuntimeError> {
        self.ledger
            .create_conversation_run(conversation, &to_record(run))?;
        Ok(())
    }

    /// Creates an immutable turn within an existing conversation run.
    ///
    /// # Errors
    /// Returns an error when the conversation/run relationship or sequence is invalid.
    pub fn create_turn(&self, turn: &TurnRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_turn(turn)?)
    }

    /// Moves a turn through its pure monotonic lifecycle policy.
    ///
    /// # Errors
    /// Returns an error when the turn is unknown, stale, or the transition is invalid.
    pub fn transition_turn(
        &self,
        turn_id: &str,
        expected: DurableTurnPhase,
        next: DurableTurnPhase,
    ) -> Result<TurnPhaseUpdate, RuntimeError> {
        if !permits_turn_transition(expected, next) {
            return Err(RuntimeError::Ledger(LedgerError::Invariant(
                "durable turn transition is not permitted".into(),
            )));
        }
        Ok(self.ledger.replace_turn_phase(turn_id, expected, next)?)
    }

    /// Lists immutable run lineage for a conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn conversation_runs(&self, conversation_id: &str) -> Result<Vec<RunRecord>, RuntimeError> {
        Ok(self.ledger.list_conversation_runs(conversation_id)?)
    }

    /// Lists durable turns in sequence order for a run.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn run_turns(&self, run_id: &str) -> Result<Vec<TurnRecord>, RuntimeError> {
        Ok(self.ledger.list_run_turns(run_id)?)
    }
}

impl<L> Coordinator<L>
where
    L: Ledger + ConversationArtifactLedger,
{
    /// Persists provenance for one title or recap generation attempt.
    ///
    /// # Errors
    /// Returns an error when the artifact lacks valid durable provenance.
    pub fn create_conversation_artifact(
        &self,
        artifact: &ConversationArtifact,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_conversation_artifact(artifact)?)
    }

    /// Lists immutable title and recap generation attempts for one conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn conversation_artifacts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationArtifact>, RuntimeError> {
        Ok(self.ledger.list_conversation_artifacts(conversation_id)?)
    }
}

impl<L> Coordinator<L>
where
    L: Ledger + ConversationLedger + ConversationArtifactLedger,
{
    /// Reads one durable conversation without exposing transcript content or provider sessions.
    ///
    /// # Errors
    /// Returns an error when durable hierarchy or artifact provenance cannot be read.
    pub fn conversation_timeline(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationTimeline, RuntimeError> {
        let runs = self
            .ledger
            .list_conversation_runs(conversation_id)?
            .into_iter()
            .map(|run| {
                Ok(ConversationTimelineRun {
                    turns: self.ledger.list_run_turns(&run.run_id)?,
                    run_id: run.run_id,
                    parent_run_id: run.parent_run_id,
                    provider: run.provider,
                })
            })
            .collect::<Result<Vec<_>, gent_ports::LedgerError>>()?;
        let artifacts = self
            .ledger
            .list_conversation_artifacts(conversation_id)?
            .iter()
            .map(artifact_summary)
            .collect();
        Ok(ConversationTimeline {
            conversation_id: conversation_id.into(),
            runs,
            artifacts,
        })
    }
}

fn artifact_summary(artifact: &ConversationArtifact) -> ConversationArtifactSummary {
    ConversationArtifactSummary {
        artifact_id: artifact.artifact_id.clone(),
        kind: artifact.kind,
        source_turn_ids: artifact.source_turn_ids.clone(),
        provider: artifact.provider.clone(),
        model_version: artifact.model_version.clone(),
        input_digest: artifact.input_digest.clone(),
        status: artifact.status,
        supersedes_artifact_id: artifact.supersedes_artifact_id.clone(),
    }
}

impl<L> Coordinator<L>
where
    L: Ledger + ConversationLedger + RunProjectionLedger,
{
    /// Resolves durable conversation lineage and optional run projections without side effects.
    ///
    /// # Errors
    /// Returns an error when the durable hierarchy or a projection cannot be read.
    pub fn conversation_status(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationStatus, RuntimeError> {
        let runs = self.ledger.list_conversation_runs(conversation_id)?;
        let mut statuses = Vec::with_capacity(runs.len());
        for run in runs {
            let live = self.ledger.find_run_projection(&run.run_id)?;
            let active_turn_id = live
                .as_ref()
                .and_then(|record| record.projection.active_turn_id.clone());
            let live_status = live.map(|record| RunLiveStatus {
                run_id: record.run_id,
                host_epoch: record.host_epoch,
                status: projected_live_status(&restore_projection(&record.projection)),
            });
            statuses.push(ConversationRunStatus {
                run_id: run.run_id,
                parent_run_id: run.parent_run_id,
                provider: run.provider,
                active_turn_id,
                live_status,
            });
        }
        Ok(ConversationStatus {
            conversation_id: conversation_id.into(),
            runs: statuses,
        })
    }
}
