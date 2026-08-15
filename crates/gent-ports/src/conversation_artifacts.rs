//! Persistence boundary for immutable title and recap provenance.

use gent_types::ConversationArtifact;

use crate::LedgerError;

/// Stores generated conversation artifacts without exposing provider sessions.
pub trait ConversationArtifactLedger: Send + Sync {
    /// Creates one immutable title or recap attempt.
    ///
    /// # Errors
    /// Returns an error when provenance is incomplete or durable state rejects it.
    fn create_conversation_artifact(
        &self,
        artifact: &ConversationArtifact,
    ) -> Result<(), LedgerError>;

    /// Lists title and recap attempts in creation order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_conversation_artifacts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationArtifact>, LedgerError>;
}
