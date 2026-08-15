//! `SQLite` persistence for user prompt content and atomic turn assignment.

use gent_ports::{ConversationPromptSave, LedgerError};
use gent_types::{ConversationMessage, ConversationPrompt};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::SqliteLedger;
use super::conversations::conversation_id_for_run;
use super::queries::{find_run, storage_error};

const MAX_PROMPT_BYTES: usize = 64 * 1024;

pub(super) fn save(
    ledger: &SqliteLedger,
    prompt: &ConversationPrompt,
) -> Result<ConversationPromptSave, LedgerError> {
    validate(prompt)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    if let Some(existing) = find_connection(&transaction, &prompt.message_id)? {
        return compare_existing(existing, prompt);
    }
    if find_run(&transaction, &prompt.run_id)?.is_none()
        || conversation_id_for_run(&transaction, &prompt.run_id)?.as_deref()
            != Some(&prompt.conversation_id)
    {
        return Err(LedgerError::Invariant(
            "prompt run must belong to its conversation".into(),
        ));
    }
    let sequence = next_sequence(&transaction, &prompt.run_id)?;
    transaction.execute(
        "INSERT INTO turns (turn_id, conversation_id, run_id, sequence, phase) VALUES (?1, ?2, ?3, ?4, 'active')",
        params![prompt.turn_id, prompt.conversation_id, prompt.run_id, sequence],
    ).map_err(storage_error)?;
    let message = message(prompt, u64::try_from(sequence).map_err(storage_error)?);
    transaction.execute(
        "INSERT INTO conversation_messages (message_id, turn_id, conversation_id, run_id, text, text_digest_sha256, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![message.message_id, message.turn_id, message.conversation_id, message.run_id, message.text, message.text_digest_sha256, message.text.len()],
    ).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(ConversationPromptSave::Created(message))
}

pub(super) fn find(
    ledger: &SqliteLedger,
    message_id: &str,
) -> Result<Option<ConversationMessage>, LedgerError> {
    let connection = ledger.lock()?;
    find_connection(&connection, message_id)
}

pub(super) fn list(
    ledger: &SqliteLedger,
    run_id: &str,
) -> Result<Vec<ConversationMessage>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection.prepare(
        "SELECT m.message_id, m.turn_id, m.conversation_id, m.run_id, t.sequence, m.text, m.text_digest_sha256 FROM conversation_messages m JOIN turns t ON t.turn_id = m.turn_id WHERE m.run_id = ?1 ORDER BY t.sequence",
    ).map_err(storage_error)?;
    statement
        .query_map([run_id], decode)
        .map_err(storage_error)?
        .collect::<Result<_, _>>()
        .map_err(storage_error)
}

fn compare_existing(
    existing: ConversationMessage,
    prompt: &ConversationPrompt,
) -> Result<ConversationPromptSave, LedgerError> {
    if existing.turn_id == prompt.turn_id
        && existing.conversation_id == prompt.conversation_id
        && existing.run_id == prompt.run_id
        && existing.text == prompt.text
    {
        Ok(ConversationPromptSave::Existing(existing))
    } else {
        Err(LedgerError::Invariant(
            "prompt message identity is immutable".into(),
        ))
    }
}

fn find_connection(
    connection: &rusqlite::Connection,
    message_id: &str,
) -> Result<Option<ConversationMessage>, LedgerError> {
    connection.query_row(
        "SELECT m.message_id, m.turn_id, m.conversation_id, m.run_id, t.sequence, m.text, m.text_digest_sha256 FROM conversation_messages m JOIN turns t ON t.turn_id = m.turn_id WHERE m.message_id = ?1",
        [message_id], decode,
    ).optional().map_err(storage_error)
}

fn next_sequence(transaction: &Transaction<'_>, run_id: &str) -> Result<i64, LedgerError> {
    transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM turns WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

fn validate(prompt: &ConversationPrompt) -> Result<(), LedgerError> {
    if [
        &prompt.message_id,
        &prompt.turn_id,
        &prompt.conversation_id,
        &prompt.run_id,
    ]
    .into_iter()
    .any(|value| !valid_identity(value))
        || prompt.text.is_empty()
        || prompt.text.len() > MAX_PROMPT_BYTES
        || prompt.text.contains('\0')
    {
        return Err(LedgerError::Invariant(
            "prompt identity or text is invalid".into(),
        ));
    }
    Ok(())
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn message(prompt: &ConversationPrompt, sequence: u64) -> ConversationMessage {
    ConversationMessage {
        message_id: prompt.message_id.clone(),
        turn_id: prompt.turn_id.clone(),
        conversation_id: prompt.conversation_id.clone(),
        run_id: prompt.run_id.clone(),
        sequence,
        text: prompt.text.clone(),
        text_digest_sha256: digest(&prompt.text),
    }
}

fn digest(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationMessage> {
    Ok(ConversationMessage {
        message_id: row.get(0)?,
        turn_id: row.get(1)?,
        conversation_id: row.get(2)?,
        run_id: row.get(3)?,
        sequence: row.get(4)?,
        text: row.get(5)?,
        text_digest_sha256: row.get(6)?,
    })
}
