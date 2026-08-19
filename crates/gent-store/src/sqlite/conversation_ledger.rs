//! Adapter joining the conversation ledger port to `SQLite` relationship queries.

use gent_ports::{ConversationLedger, LedgerError, RunRecord, TurnPhaseUpdate};
use gent_types::{ConversationListItem, ConversationRecord, DurableTurnPhase, TurnRecord};

use super::{SqliteLedger, conversations};

impl ConversationLedger for SqliteLedger {
    fn list_conversations(&self) -> Result<Vec<ConversationListItem>, LedgerError> {
        conversations::list_conversations(self)
    }
    fn create_conversation_run(
        &self,
        conversation: &ConversationRecord,
        run: &RunRecord,
    ) -> Result<(), LedgerError> {
        conversations::create_conversation_run(self, conversation, run)
    }

    fn find_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationRecord>, LedgerError> {
        conversations::find_conversation(self, conversation_id)
    }

    fn create_turn(&self, turn: &TurnRecord) -> Result<(), LedgerError> {
        conversations::create_turn(self, turn)
    }

    fn find_turn(&self, turn_id: &str) -> Result<Option<TurnRecord>, LedgerError> {
        conversations::find_turn(self, turn_id)
    }

    fn list_conversation_runs(&self, conversation_id: &str) -> Result<Vec<RunRecord>, LedgerError> {
        conversations::list_conversation_runs(self, conversation_id)
    }

    fn list_run_turns(&self, run_id: &str) -> Result<Vec<TurnRecord>, LedgerError> {
        conversations::list_run_turns(self, run_id)
    }

    fn replace_turn_phase(
        &self,
        turn_id: &str,
        expected: DurableTurnPhase,
        next: DurableTurnPhase,
    ) -> Result<TurnPhaseUpdate, LedgerError> {
        conversations::replace_turn_phase(self, turn_id, expected, next)
    }
}

impl gent_ports::ConversationArtifactLedger for SqliteLedger {
    fn create_conversation_artifact(
        &self,
        artifact: &gent_types::ConversationArtifact,
    ) -> Result<(), LedgerError> {
        super::conversation_artifacts::create(self, artifact)
    }

    fn list_conversation_artifacts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<gent_types::ConversationArtifact>, LedgerError> {
        super::conversation_artifacts::list(self, conversation_id)
    }
}
