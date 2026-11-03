//! Notice-only reporting for owned launch errors; never changes quarantine state.
use super::helpers::valid_owner;
use super::{SqliteLedger, require_open};
use crate::sqlite::queries::storage_error;
use gent_ports::LedgerError;
use gent_types::HostEpoch;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub(super) fn record(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator: &str,
    epoch: HostEpoch,
    error: &str,
    failure_code: Option<gent_types::AgentChatFailureCode>,
) -> Result<(), LedgerError> {
    valid_owner(coordinator)?;
    if message_id.is_empty()
        || message_id.len() > 1024
        || error.is_empty()
        || error.len() > 64 * 1024
        || error.contains('\0')
    {
        return Err(LedgerError::Invariant(
            "owned launch failure notice is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, epoch)?;
    let scope:Option<(String,String,String)>=transaction.query_row(
        "SELECT m.conversation_id,m.run_id,m.turn_id FROM conversation_messages m JOIN agent_chat_prompt_dispatches d ON d.message_id=m.message_id WHERE m.message_id=?1 AND d.coordinator_id=?2 AND d.host_epoch=?3 AND d.state IN ('launching','unprovable')",
        params![message_id,coordinator,epoch.0],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(storage_error)?;
    let (conversation, run, turn) = scope.ok_or_else(|| {
        LedgerError::Invariant("launch failure notice is not owned by this coordinator".into())
    })?;
    let existing:bool=transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_chat_transcript_events WHERE conversation_id=?1 AND turn_id=?2 AND kind='notice')",
        params![conversation,turn],|row|row.get(0)).map_err(storage_error)?;
    if existing {
        return transaction.commit().map_err(storage_error);
    }
    let cursor:u64=transaction.query_row("SELECT COALESCE(MAX(cursor),0)+1 FROM agent_chat_transcript_events WHERE conversation_id=?1",[&conversation],|row|row.get(0)).map_err(storage_error)?;
    transaction.execute(
        "INSERT INTO agent_chat_transcript_events(conversation_id,cursor,event_id,turn_id,run_id,kind,text,is_partial) VALUES(?1,?2,?3,?4,?5,'notice',?6,0)",
        params![conversation,cursor,format!("{}{message_id}", failure_code.map_or("launch-failure-notice:", gent_types::AgentChatFailureCode::event_prefix)),turn,run,error]).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}
