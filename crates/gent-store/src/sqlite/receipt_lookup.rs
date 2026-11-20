use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptDisposition, HostEpoch, Receipt, ReceiptId, ReceiptIntentKind, ReceiptLookup,
    ReceiptPromptOutcome,
};
use rusqlite::OptionalExtension;

use super::SqliteLedger;
use super::queries::{decode_status, storage_error};

impl SqliteLedger {
    pub fn lookup_receipt(&self, receipt_id: &ReceiptId) -> Result<ReceiptLookup, LedgerError> {
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT r.idempotency_key, r.status, r.host_epoch, r.kind, p.conversation_id, p.run_id, p.turn_id, p.message_id, p.disposition FROM receipts r LEFT JOIN agent_chat_prompt_receipts p ON p.idempotency_key = r.idempotency_key WHERE r.receipt_id = ?1",
                [&receipt_id.0],
                |row| {
                    let receipt = Receipt {
                        receipt_id: receipt_id.clone(),
                        idempotency_key: row.get(0)?,
                        status: decode_status(&row.get::<_, String>(1)?)?,
                        host_epoch: HostEpoch(row.get(2)?),
                    };
                    let prompt = match row.get::<_, Option<String>>(4)? {
                        Some(conversation_id) => Some(ReceiptPromptOutcome {
                            conversation_id,
                            run_id: row.get(5)?,
                            turn_id: row.get(6)?,
                            message_id: row.get(7)?,
                            delivery: match row.get::<_, String>(8)?.as_str() {
                                "queue" => AgentChatPromptDisposition::Queue,
                                _ => AgentChatPromptDisposition::Send,
                            }
                            .delivery(),
                        }),
                        None => None,
                    };
                    Ok((receipt, row.get::<_, String>(3)?, prompt))
                },
            )
            .optional()
            .map_err(storage_error)?;
        let Some((receipt, kind, prompt)) = row else {
            return Ok(ReceiptLookup::NeverSeen);
        };
        let intent = ReceiptIntentKind::from_command_kind(&kind).ok_or_else(|| {
            LedgerError::Invariant(
                "the receipt belongs to an intent without a receipt lookup".into(),
            )
        })?;
        Ok(ReceiptLookup::found(intent, receipt, prompt))
    }
}

#[cfg(test)]
#[path = "receipt_lookup_tests.rs"]
mod tests;
