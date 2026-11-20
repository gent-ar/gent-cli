use gent_ports::LedgerError;
use gent_types::{
    AgentChatConversationId, Command, ConversationActivityFact, ConversationActivityScope,
    DurableTurnPhase, HostEpoch, Receipt, ReceiptId, ReceiptStatus,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::agent_chat_queue_activity::{append, receipted};
use super::queries::{find_receipt, insert_receipt, storage_error};
use super::{SqliteLedger, transcript_attachments, turn_terminal};

const WITHDRAW_ON_RETURN_KEY_PREFIX: &str = "agent-chat-withdraw-on-return:";
const CANCEL_ON_RETURN_KEY_PREFIX: &str = "agent-chat-cancel-on-return:";
const GOAL_CONTINUATION: &str = "EXISTS(SELECT 1 FROM agent_chat_transcript_events te WHERE te.event_id = 'user:' || d.message_id AND json_extract(te.origin_json, '$.kind') = 'goalContinuation')";

pub(super) fn stop(
    ledger: &SqliteLedger,
    command: &Command,
    conversation_id: &AgentChatConversationId,
) -> Result<Receipt, LedgerError> {
    let conflict = LedgerError::Invariant("interrupt receipt is bound to another command".into());
    receipted(ledger, command, conflict, |transaction| {
        let held = messages(
            transaction,
            &format!(
                "SELECT d.message_id, {GOAL_CONTINUATION} FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN agent_chat_prompt_receipts p ON p.message_id = d.message_id WHERE m.conversation_id = ?1 AND p.disposition = 'queue' AND d.state IN ('awaiting_readiness', 'provisioning', 'pending') ORDER BY d.created_rowid"
            ),
            &conversation_id.0,
        )?;
        for (message_id, continuation) in held {
            retire(transaction, &message_id, command.host_epoch, continuation)?;
        }
        let written = messages(
            transaction,
            "SELECT s.message_id, 0 FROM agent_chat_steer_targets s JOIN conversation_messages m ON m.message_id = s.message_id WHERE m.conversation_id = ?1 AND s.state = 'claimed'",
            &conversation_id.0,
        )?;
        for (message_id, _) in written {
            mark(
                transaction,
                OnReturn::Withdraw,
                &message_id,
                command.host_epoch,
            )?;
        }
        request_interrupts(transaction, command, conversation_id)
    })
}

fn request_interrupts(
    transaction: &Transaction<'_>,
    command: &Command,
    conversation_id: &AgentChatConversationId,
) -> Result<(), LedgerError> {
    let mut statement = transaction
        .prepare("SELECT t.run_id, t.turn_id FROM turns t WHERE t.conversation_id = ?1 AND t.phase IN ('active', 'waitingPermission', 'waitingQuestion') AND NOT EXISTS(SELECT 1 FROM agent_chat_steer_targets s JOIN conversation_messages m ON m.message_id = s.message_id WHERE m.turn_id = t.turn_id AND s.state = 'claimed') ORDER BY t.rowid")
        .map_err(storage_error)?;
    let turns = statement
        .query_map([&conversation_id.0], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(storage_error)?
        .collect::<Result<Vec<(String, String)>, _>>()
        .map_err(storage_error)?;
    for (run_id, turn_id) in turns {
        append(
            transaction,
            format!("agent-chat-interrupt:{}:{turn_id}", command.receipt_id.0),
            command.receipt_id.clone(),
            ConversationActivityFact::InterruptRequested {
                scope: ConversationActivityScope {
                    conversation_id: conversation_id.0.clone(),
                    run_id,
                    turn_id,
                    host_epoch: command.host_epoch,
                    cursor: 0,
                },
            },
        )?;
    }
    Ok(())
}

pub(super) fn withdraw_if_marked(
    transaction: &Transaction<'_>,
    message_id: &str,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    for on_return in [OnReturn::Cancel, OnReturn::Withdraw] {
        if find_receipt(transaction, &on_return.key(message_id))?.is_some() {
            return retire(
                transaction,
                message_id,
                host_epoch,
                on_return == OnReturn::Cancel,
            );
        }
    }
    Ok(())
}

pub(super) fn cancel_when_returned(
    transaction: &Transaction<'_>,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
    receipt_id: &ReceiptId,
    host_epoch: HostEpoch,
) -> Result<bool, LedgerError> {
    let target = transaction
        .query_row(
            "SELECT s.provider, t.run_id, t.turn_id FROM agent_chat_steer_targets t JOIN conversation_messages m ON m.message_id = t.message_id JOIN agent_chat_run_selections s ON s.run_id = t.run_id WHERE t.message_id = ?1 AND m.conversation_id = ?2 AND t.state = 'claimed'",
            params![message_id, conversation_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((provider, run_id, turn_id)) = target else {
        return Ok(false);
    };
    if !super::agent_chat_read_ledger::provider(&provider)?.withdraws_joined_prompts() {
        return Ok(false);
    }
    if find_receipt(transaction, &OnReturn::Cancel.key(message_id))?.is_some() {
        return Ok(true);
    }
    mark(transaction, OnReturn::Cancel, message_id, host_epoch)?;
    append(
        transaction,
        format!("agent-chat-queue:{message_id}:cancel-requested"),
        receipt_id.clone(),
        ConversationActivityFact::PromptCancelRequested {
            scope: ConversationActivityScope {
                conversation_id: conversation_id.0.clone(),
                run_id,
                turn_id,
                host_epoch,
                cursor: 0,
            },
            message_id: message_id.into(),
            receipt_id: receipt_id.0.clone(),
        },
    )?;
    Ok(true)
}

pub(super) fn canceled_written(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<String>, LedgerError> {
    let mut statement = connection
        .prepare_cached("SELECT t.message_id FROM agent_chat_steer_targets t WHERE t.run_id = ?1 AND t.state = 'claimed' AND EXISTS(SELECT 1 FROM receipts r WHERE r.idempotency_key = ?2 || t.message_id)")
        .map_err(storage_error)?;
    statement
        .query_map(params![run_id, CANCEL_ON_RETURN_KEY_PREFIX], |row| {
            row.get(0)
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OnReturn {
    Withdraw,
    Cancel,
}

impl OnReturn {
    fn key(self, message_id: &str) -> String {
        match self {
            Self::Withdraw => format!("{WITHDRAW_ON_RETURN_KEY_PREFIX}{message_id}"),
            Self::Cancel => format!("{CANCEL_ON_RETURN_KEY_PREFIX}{message_id}"),
        }
    }
}

fn messages(
    db: &Connection,
    query: &str,
    conversation_id: &str,
) -> Result<Vec<(String, bool)>, LedgerError> {
    let mut statement = db.prepare(query).map_err(storage_error)?;
    statement
        .query_map([conversation_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

fn mark(
    db: &Connection,
    on_return: OnReturn,
    message_id: &str,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let key = on_return.key(message_id);
    if find_receipt(db, &key)?.is_some() {
        return Ok(());
    }
    let receipt = Receipt {
        receipt_id: ReceiptId(key.clone()),
        idempotency_key: key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch,
    };
    let command = Command {
        receipt_id: receipt.receipt_id.clone(),
        idempotency_key: key,
        host_epoch,
        kind: match on_return {
            OnReturn::Withdraw => "agentChatWithdrawOnReturn",
            OnReturn::Cancel => "agentChatCancelOnReturn",
        }
        .into(),
        payload: serde_json::json!({ "messageId": message_id }),
    };
    insert_receipt(db, &receipt, &command)
}

fn retire(
    transaction: &Transaction<'_>,
    message_id: &str,
    host_epoch: HostEpoch,
    canceled: bool,
) -> Result<(), LedgerError> {
    let prompt = super::agent_chat_ledger::saved_prompt(transaction, message_id)?;
    transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'settled', coordinator_id = NULL, host_epoch = NULL WHERE message_id = ?1",
            params![message_id],
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "DELETE FROM agent_chat_steer_targets WHERE message_id = ?1",
            [message_id],
        )
        .map_err(storage_error)?;
    turn_terminal::settle(
        transaction,
        &prompt.message.turn_id,
        host_epoch,
        DurableTurnPhase::Cancelled,
    )?;
    let scope = ConversationActivityScope {
        conversation_id: prompt.message.conversation_id.clone(),
        run_id: prompt.message.run_id.clone(),
        turn_id: prompt.message.turn_id.clone(),
        host_epoch,
        cursor: 0,
    };
    let receipt_id = prompt.receipt.receipt_id.clone();
    if canceled {
        return append(
            transaction,
            format!("agent-chat-queue:{message_id}:canceled"),
            receipt_id,
            ConversationActivityFact::PromptCanceled {
                scope,
                message_id: message_id.into(),
            },
        );
    }
    append(
        transaction,
        format!("agent-chat-queue:{message_id}:withdrawn"),
        receipt_id.clone(),
        ConversationActivityFact::PromptWithdrawn {
            scope,
            message_id: message_id.into(),
            receipt_id: receipt_id.0,
            text: prompt.message.text,
            attachments: transcript_attachments::turn_references(
                transaction,
                &prompt.message.turn_id,
            )?,
        },
    )
}
