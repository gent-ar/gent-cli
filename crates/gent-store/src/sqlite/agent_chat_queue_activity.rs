use gent_ports::{AgentChatQueuedPromptLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatRejection, AgentChatRunId, Command, ConversationActivityFact,
    ConversationActivityScope, Event, HostEpoch, Receipt, ReceiptId, ReceiptStatus,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::epoch::require_epoch;
use super::queries::{
    append_event, find_receipt, host_ingress, insert_receipt, receipt_matches_command,
    storage_error,
};
use super::{SqliteLedger, conversation_activity_ledger};

pub(super) const STEER_KEY_PREFIX: &str = "agent-chat-steer-queued-prompt:";

impl AgentChatQueuedPromptLedger for SqliteLedger {
    fn cancel_queued_agent_chat_prompt(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        message_id: &str,
    ) -> Result<Receipt, LedgerError> {
        let command = queue_command(
            receipt_id,
            host_epoch,
            conversation_id,
            message_id,
            format!("agent-chat-cancel-queued-prompt:{}", receipt_id.0),
            "agentChatCancelQueuedPrompt",
        )?;
        let conflict =
            LedgerError::Invariant("queued prompt receipt is bound to another prompt".into());
        receipted(self, &command, conflict, |transaction| {
            let (run_id, turn_id) =
                interrupt_queued_turn(transaction, conversation_id, message_id, host_epoch)?;
            append(
                transaction,
                format!("agent-chat-queue:{message_id}:canceled"),
                receipt_id.clone(),
                ConversationActivityFact::PromptCanceled {
                    scope: ConversationActivityScope {
                        conversation_id: conversation_id.0.clone(),
                        run_id,
                        turn_id,
                        host_epoch,
                        cursor: 0,
                    },
                    message_id: message_id.to_owned(),
                },
            )
        })
    }

    fn steer_queued_agent_chat_prompt(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        message_id: &str,
    ) -> Result<(Receipt, AgentChatRunId), LedgerError> {
        let command = queue_command(
            receipt_id,
            host_epoch,
            conversation_id,
            message_id,
            format!("{STEER_KEY_PREFIX}{message_id}"),
            "agentChatSteerQueuedPrompt",
        )?;
        let not_steerable = || LedgerError::Rejected(AgentChatRejection::QueuedPromptNotSteerable);
        let receipt = receipted(self, &command, not_steerable(), |transaction| {
            queued_prompt(transaction, conversation_id, message_id)?
                .map(|_| ())
                .ok_or_else(not_steerable)
        })?;
        let run_id = self
            .lock()?
            .query_row(
                "SELECT run_id FROM conversation_messages WHERE message_id = ?1",
                [message_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(storage_error)?;
        Ok((receipt, AgentChatRunId(run_id)))
    }

    fn interrupt_active_turn_for_steer(
        &self,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<bool, LedgerError> {
        super::agent_chat_steer_interrupt::mark(self, host_epoch, conversation_id, run_id)
    }
}

fn queue_command(
    receipt_id: &ReceiptId,
    host_epoch: HostEpoch,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
    idempotency_key: String,
    kind: &str,
) -> Result<Command, LedgerError> {
    if [
        receipt_id.0.as_str(),
        conversation_id.0.as_str(),
        message_id,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(LedgerError::Invariant(
            "queued prompt command identity is invalid".into(),
        ));
    }
    Ok(Command {
        receipt_id: receipt_id.clone(),
        idempotency_key,
        host_epoch,
        kind: kind.into(),
        payload: serde_json::json!({ "conversationId": conversation_id.0, "messageId": message_id }),
    })
}

fn receipted(
    ledger: &SqliteLedger,
    command: &Command,
    conflict: LedgerError,
    apply: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<(), LedgerError>,
) -> Result<Receipt, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(command.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
        return receipt_matches_command(&transaction, command)?
            .then_some(receipt)
            .ok_or(conflict);
    }
    let receipt_owner = transaction
        .query_row(
            "SELECT 1 FROM receipts WHERE receipt_id = ?1",
            [&command.receipt_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    if receipt_owner.is_some() {
        return Err(LedgerError::Invariant(
            "queued prompt receipt id is owned by another command".into(),
        ));
    }
    apply(&transaction)?;
    let receipt = Receipt {
        receipt_id: command.receipt_id.clone(),
        idempotency_key: command.idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: command.host_epoch,
    };
    insert_receipt(&transaction, &receipt, command)?;
    transaction.commit().map_err(storage_error)?;
    Ok(receipt)
}

fn queued_prompt(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
) -> Result<Option<(String, String)>, LedgerError> {
    connection
        .query_row(
            "SELECT m.run_id, m.turn_id FROM agent_chat_prompt_dispatches d JOIN agent_chat_prompt_receipts p ON p.message_id = d.message_id JOIN conversation_messages m ON m.message_id = d.message_id WHERE d.message_id = ?1 AND m.conversation_id = ?2 AND p.disposition = 'queue' AND d.state IN ('awaiting_readiness', 'pending')",
            params![message_id, conversation_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)
}

fn interrupt_queued_turn(
    transaction: &rusqlite::Transaction<'_>,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
    host_epoch: HostEpoch,
) -> Result<(String, String), LedgerError> {
    let connection: &Connection = transaction;
    let (run_id, turn_id) = queued_prompt(connection, conversation_id, message_id)?.ok_or(
        LedgerError::Rejected(AgentChatRejection::QueuedPromptNotCancelable),
    )?;
    connection
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'settled', coordinator_id = NULL, host_epoch = NULL WHERE message_id = ?1 AND state IN ('awaiting_readiness', 'pending')",
            [message_id],
        )
        .map_err(storage_error)?;
    if !super::turn_terminal::settle(
        transaction,
        &turn_id,
        host_epoch,
        gent_types::DurableTurnPhase::Cancelled,
    )? {
        return Err(LedgerError::Invariant(
            "queued prompt turn is no longer active".into(),
        ));
    }
    Ok((run_id, turn_id))
}

pub(super) fn append(
    connection: &Connection,
    event_id: String,
    receipt_id: ReceiptId,
    fact: ConversationActivityFact,
) -> Result<(), LedgerError> {
    append_activity(
        connection,
        event_id,
        receipt_id,
        "agentChatQueueActivity",
        fact,
    )
}

pub(super) fn append_activity(
    connection: &Connection,
    event_id: String,
    receipt_id: ReceiptId,
    kind: &str,
    fact: ConversationActivityFact,
) -> Result<(), LedgerError> {
    let scope = fact.scope();
    let event = append_event(
        connection,
        &Event {
            cursor: 0,
            event_id,
            receipt_id,
            host_epoch: scope.host_epoch,
            kind: kind.into(),
            payload: serde_json::json!({
                "conversationId": scope.conversation_id,
                "runId": scope.run_id,
                "turnId": scope.turn_id,
                "activity": fact,
            }),
        },
    )?;
    let fact = gent_core::with_activity_cursor(fact, event.cursor);
    conversation_activity_ledger::append(connection, &fact)?;
    connection
        .execute(
            "INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES (?1, ?2, 'activity', ?3)",
            params![
                fact.scope().conversation_id,
                format!("activity:{}", event.event_id),
                serde_json::json!({
                    "conversationId": fact.scope().conversation_id,
                    "runId": fact.scope().run_id,
                    "turnId": fact.scope().turn_id,
                    "activity": fact,
                })
                .to_string(),
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}
