use gent_core::permits_turn_transition;
use gent_ports::{
    ConversationArtifactLedger, ConversationLedger, Ledger, LedgerError, RunCheckpointLedger,
    RunLifecycleFactLedger, RunRecord, TurnPhaseUpdate,
};
use gent_types::{
    ConversationArtifact, ConversationArtifactSummary, ConversationListItem, ConversationRecord,
    ConversationRunStatus, ConversationStatus, ConversationTimeline, ConversationTimelineRun,
    DurableTurnPhase, TurnRecord,
};

use crate::{Coordinator, RuntimeError, to_record};

impl<L> Coordinator<L>
where
    L: Ledger + ConversationLedger,
{
    pub fn conversations(&self) -> Result<Vec<ConversationListItem>, RuntimeError> {
        Ok(self.ledger.list_conversations()?)
    }
    pub fn create_conversation_run(
        &self,
        conversation: &ConversationRecord,
        run: &gent_core::Run,
    ) -> Result<(), RuntimeError> {
        self.ledger
            .create_conversation_run(conversation, &to_record(run))?;
        Ok(())
    }

    pub fn create_turn(&self, turn: &TurnRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_turn(turn)?)
    }

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

    pub fn conversation_runs(&self, conversation_id: &str) -> Result<Vec<RunRecord>, RuntimeError> {
        Ok(self.ledger.list_conversation_runs(conversation_id)?)
    }

    pub fn run_turns(&self, run_id: &str) -> Result<Vec<TurnRecord>, RuntimeError> {
        Ok(self.ledger.list_run_turns(run_id)?)
    }
}

impl<L> Coordinator<L>
where
    L: Ledger + ConversationArtifactLedger,
{
    pub fn create_conversation_artifact(
        &self,
        artifact: &ConversationArtifact,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_conversation_artifact(artifact)?)
    }

    pub fn conversation_artifacts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationArtifact>, RuntimeError> {
        Ok(self.ledger.list_conversation_artifacts(conversation_id)?)
    }
}

impl<L> Coordinator<L>
where
    L: Ledger + ConversationLedger + ConversationArtifactLedger + RunCheckpointLedger,
{
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
                    checkpoints: self.ledger.list_run_checkpoints(&run.run_id)?,
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
    L: Ledger + ConversationLedger + RunLifecycleFactLedger,
{
    pub fn conversation_status(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationStatus, RuntimeError> {
        let runs = self
            .ledger
            .list_conversation_runs(conversation_id)?
            .into_iter()
            .map(|run| {
                Ok(ConversationRunStatus {
                    active_turn_id: self.ledger.run_active_turn(&run.run_id)?,
                    live_status: self.ledger.run_live_status(&run.run_id)?,
                    run_id: run.run_id,
                    parent_run_id: run.parent_run_id,
                    provider: run.provider,
                })
            })
            .collect::<Result<Vec<_>, LedgerError>>()?;
        Ok(ConversationStatus {
            conversation_id: conversation_id.into(),
            runs,
        })
    }
}
