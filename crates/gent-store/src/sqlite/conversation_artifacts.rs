//! `SQLite` persistence for title and recap provenance records.

use gent_ports::LedgerError;
use gent_types::{ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn create(
    ledger: &SqliteLedger,
    artifact: &ConversationArtifact,
) -> Result<(), LedgerError> {
    validate(artifact)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let conversation_exists = transaction
        .query_row(
            "SELECT 1 FROM conversations WHERE conversation_id = ?1",
            [&artifact.conversation_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?
        .is_some();
    if !conversation_exists {
        return Err(LedgerError::Invariant(
            "artifact conversation does not exist".into(),
        ));
    }
    if let Some(previous) = &artifact.supersedes_artifact_id {
        let matches = transaction
            .query_row(
                "SELECT kind, conversation_id FROM conversation_artifacts WHERE artifact_id = ?1",
                [previous],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(storage_error)?;
        if matches != Some((kind(artifact.kind).into(), artifact.conversation_id.clone())) {
            return Err(LedgerError::Invariant(
                "superseded artifact must share kind and conversation".into(),
            ));
        }
    }
    let turns = serde_json::to_string(&artifact.source_turn_ids).map_err(storage_error)?;
    transaction.execute(
        "INSERT INTO conversation_artifacts (artifact_id, conversation_id, kind, source_turn_ids, provider, model_version, input_digest, status, text, supersedes_artifact_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![artifact.artifact_id, artifact.conversation_id, kind(artifact.kind), turns, artifact.provider, artifact.model_version, artifact.input_digest, status(artifact.status), artifact.text, artifact.supersedes_artifact_id],
    ).map_err(storage_error)?;
    if let Some(previous) = &artifact.supersedes_artifact_id {
        transaction
            .execute(
                "UPDATE conversation_artifacts SET status = 'superseded', text = NULL WHERE artifact_id = ?1",
                [previous],
            )
            .map_err(storage_error)?;
    }
    transaction.commit().map_err(storage_error)
}

pub(super) fn list(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<Vec<ConversationArtifact>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection.prepare("SELECT artifact_id, conversation_id, kind, source_turn_ids, provider, model_version, input_digest, status, text, supersedes_artifact_id FROM conversation_artifacts WHERE conversation_id = ?1 ORDER BY rowid").map_err(storage_error)?;
    statement
        .query_map([conversation_id], decode)
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

fn validate(artifact: &ConversationArtifact) -> Result<(), LedgerError> {
    if artifact.artifact_id.is_empty()
        || artifact.source_turn_ids.is_empty()
        || artifact.provider.is_empty()
        || artifact.model_version.is_empty()
        || artifact.input_digest.is_empty()
    {
        return Err(LedgerError::Invariant(
            "artifact provenance must be complete".into(),
        ));
    }
    if matches!(artifact.status, ConversationArtifactStatus::Completed) != artifact.text.is_some() {
        return Err(LedgerError::Invariant(
            "completed artifact requires text and other states omit it".into(),
        ));
    }
    Ok(())
}

const fn kind(value: ConversationArtifactKind) -> &'static str {
    match value {
        ConversationArtifactKind::Title => "title",
        ConversationArtifactKind::Recap => "recap",
    }
}
const fn status(value: ConversationArtifactStatus) -> &'static str {
    match value {
        ConversationArtifactStatus::Pending => "pending",
        ConversationArtifactStatus::Completed => "completed",
        ConversationArtifactStatus::Failed => "failed",
        ConversationArtifactStatus::Superseded => "superseded",
    }
}
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationArtifact> {
    let source_turn_ids = serde_json::from_str(&row.get::<_, String>(3)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(ConversationArtifact {
        artifact_id: row.get(0)?,
        conversation_id: row.get(1)?,
        kind: match row.get::<_, String>(2)?.as_str() {
            "title" => ConversationArtifactKind::Title,
            "recap" => ConversationArtifactKind::Recap,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        source_turn_ids,
        provider: row.get(4)?,
        model_version: row.get(5)?,
        input_digest: row.get(6)?,
        status: match row.get::<_, String>(7)?.as_str() {
            "pending" => ConversationArtifactStatus::Pending,
            "completed" => ConversationArtifactStatus::Completed,
            "failed" => ConversationArtifactStatus::Failed,
            "superseded" => ConversationArtifactStatus::Superseded,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        text: row.get(8)?,
        supersedes_artifact_id: row.get(9)?,
    })
}
