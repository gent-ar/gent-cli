use gent_ports::LedgerError;
use gent_types::AgentChatPromptSaved;
use rusqlite::params;

use super::super::super::{SqliteLedger, queries::storage_error};

pub(super) fn read(
    ledger: &SqliteLedger,
    run_id: Option<&str>,
    after_message_id: &str,
    limit: u16,
) -> Result<Vec<AgentChatPromptSaved>, LedgerError> {
    if limit == 0 || limit > 128 || after_message_id.len() > 512 {
        return Err(LedgerError::Invariant("held prompt page is invalid".into()));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection.transaction().map_err(storage_error)?;
    let messages = {
        let mut statement = transaction.prepare(
            "SELECT m.message_id FROM agent_chat_prompt_dispatches d
             JOIN conversation_messages m ON m.message_id=d.message_id
             JOIN agent_chat_prompt_receipts p ON p.message_id=m.message_id
              AND p.run_id=m.run_id AND p.turn_id=m.turn_id AND p.conversation_id=m.conversation_id
             JOIN receipts r ON r.idempotency_key=p.idempotency_key AND r.status='settled'
             JOIN turns t ON t.turn_id=p.turn_id AND t.run_id=p.run_id AND t.conversation_id=p.conversation_id
             JOIN agent_chat_run_selections s ON s.run_id=p.run_id
             WHERE d.state='awaiting_readiness' AND t.phase='active'
              AND p.disposition IN ('send','queue') AND m.message_id>?1
              AND (?3 IS NULL OR p.run_id=?3)
              AND NOT EXISTS(SELECT 1 FROM agent_chat_steer_targets target WHERE target.message_id=m.message_id)
              AND p.run_id=(SELECT current.run_id FROM runs current
               JOIN agent_chat_run_selections selected ON selected.run_id=current.run_id
               WHERE current.conversation_id=p.conversation_id ORDER BY current.rowid DESC LIMIT 1)
             ORDER BY m.message_id LIMIT ?2",
        ).map_err(storage_error)?;
        let rows = statement
            .query_map(params![after_message_id, limit, run_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
    };
    messages
        .iter()
        .map(|message| super::helpers::saved(&transaction, message))
        .collect()
}

#[cfg(test)]
#[path = "prompt_dispatch_awaiting_tests.rs"]
mod tests;
