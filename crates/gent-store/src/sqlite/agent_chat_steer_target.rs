use super::queries::storage_error;
use super::{SqliteLedger, agent_chat_queue_activity as queue};
use gent_ports::LedgerError;
use gent_types::{
    AgentChatConversationId, AgentChatRejection, AgentChatRunId, ConversationActivityFact,
    ConversationActivityScope, HostEpoch, Receipt, ReceiptId,
};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn request(
    ledger: &SqliteLedger,
    receipt_id: &ReceiptId,
    epoch: HostEpoch,
    conversation: &AgentChatConversationId,
    message: &str,
    run: &AgentChatRunId,
    turn: &str,
) -> Result<(Receipt, AgentChatRunId), LedgerError> {
    if run.0.trim().is_empty() || turn.trim().is_empty() {
        return Err(rejected());
    }
    let mut command = queue::queue_command(
        receipt_id,
        epoch,
        conversation,
        message,
        format!(
            "agent-chat-steer-for-turn:{}",
            serde_json::json!([conversation.0, message, run.0, turn, receipt_id.0])
        ),
        "agentChatSteerQueuedPromptForTurn",
    )?;
    command.payload["expectedRunId"] = run.0.clone().into();
    command.payload["expectedTurnId"] = turn.into();
    let receipt = queue::receipted(ledger, &command, rejected(), |tx| {
        queue::queued_prompt(tx, conversation, message)?.ok_or_else(rejected)?;
        if !live(tx, &conversation.0, &run.0, turn, epoch)? {
            return Err(rejected());
        }
        let busy = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_chat_steer_targets WHERE message_id=?1 AND state IN ('requested','claimed'))", [message], |r| r.get::<_, bool>(0)).map_err(storage_error)?;
        if busy {
            return Err(rejected());
        }
        bind(tx, receipt_id, epoch, conversation, message, run, turn)
    })?;
    Ok((receipt, run.clone()))
}

pub(super) fn bind(
    db: &Connection,
    receipt_id: &ReceiptId,
    epoch: HostEpoch,
    conversation: &AgentChatConversationId,
    message: &str,
    run: &AgentChatRunId,
    turn: &str,
) -> Result<(), LedgerError> {
    let provider: String = db
        .query_row(
            "SELECT provider FROM agent_chat_run_selections WHERE run_id=?1",
            [&run.0],
            |r| r.get(0),
        )
        .map_err(storage_error)?;
    if super::agent_chat_read_ledger::provider(&provider)?.steers_by_interrupt() {
        super::agent_chat_steer_interrupt::record(db, epoch, conversation, run, turn)?;
    } else {
        db.execute("INSERT INTO agent_chat_steer_targets(message_id,receipt_id,run_id,turn_id,state) VALUES (?1,?2,?3,?4,'requested') ON CONFLICT(message_id) DO UPDATE SET receipt_id=excluded.receipt_id,run_id=excluded.run_id,turn_id=excluded.turn_id,state='requested'",
            params![message, receipt_id.0, run.0, turn]).map_err(storage_error)?;
    }
    queue::append(
        db,
        format!("agent-chat-steer:{}:requested", receipt_id.0),
        receipt_id.clone(),
        ConversationActivityFact::PromptSteerRequested {
            scope: ConversationActivityScope {
                conversation_id: conversation.0.clone(),
                run_id: run.0.clone(),
                turn_id: turn.into(),
                host_epoch: epoch,
                cursor: 0,
            },
            message_id: message.into(),
            receipt_id: receipt_id.0.clone(),
        },
    )
}

pub(super) fn live(
    db: &Connection,
    conversation: &str,
    run: &str,
    turn: &str,
    epoch: HostEpoch,
) -> Result<bool, LedgerError> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM conversation_messages m JOIN agent_chat_prompt_dispatches d ON d.message_id=m.message_id JOIN turns t ON t.turn_id=m.turn_id WHERE m.conversation_id=?1 AND m.run_id=?2 AND m.turn_id=?3 AND d.state IN ('launching','started') AND d.host_epoch=?4 AND t.phase IN ('active','waitingPermission','waitingQuestion') AND NOT EXISTS(SELECT 1 FROM conversation_activity_facts a WHERE a.conversation_id=m.conversation_id AND a.run_id=m.run_id AND json_extract(a.payload,'$.turnId')=m.turn_id AND json_extract(a.payload,'$.type')='terminal'))",
        params![conversation,run,turn,epoch.0], |r| r.get(0)).map_err(storage_error)
}

pub(super) fn target(
    db: &Connection,
    message: &str,
) -> Result<Option<(String, String, String, String)>, LedgerError> {
    db.query_row(
        "SELECT receipt_id,run_id,turn_id,state FROM agent_chat_steer_targets WHERE message_id=?1",
        [message],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .optional()
    .map_err(storage_error)
}

pub(super) fn expire(
    db: &Connection,
    message: &str,
    epoch: HostEpoch,
    reason: &str,
) -> Result<(), LedgerError> {
    let Some((receipt, run, turn, state)) = target(db, message)? else {
        return Ok(());
    };
    if !matches!(state.as_str(), "requested" | "claimed") {
        return Ok(());
    }
    let conversation = db
        .query_row(
            "SELECT conversation_id FROM conversation_messages WHERE message_id=?1",
            [message],
            |r| r.get::<_, String>(0),
        )
        .map_err(storage_error)?;
    db.execute(
        "UPDATE agent_chat_steer_targets SET state='rejected' WHERE message_id=?1",
        [message],
    )
    .map_err(storage_error)?;
    queue::append(
        db,
        format!("agent-chat-steer:{receipt}:rejected"),
        ReceiptId(receipt.clone()),
        ConversationActivityFact::PromptSteerRejected {
            scope: ConversationActivityScope {
                conversation_id: conversation,
                run_id: run,
                turn_id: turn,
                host_epoch: epoch,
                cursor: 0,
            },
            message_id: message.into(),
            receipt_id: receipt,
            reason: reason.into(),
        },
    )
}

fn rejected() -> LedgerError {
    LedgerError::Rejected(AgentChatRejection::QueuedPromptNotSteerable)
}

pub(super) fn expire_turn(
    db: &Connection,
    turn: &str,
    epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let mut statement = db.prepare("SELECT message_id FROM agent_chat_steer_targets WHERE turn_id=?1 AND state='requested'").map_err(storage_error)?;
    let messages = statement
        .query_map([turn], |r| r.get::<_, String>(0))
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    for message in messages {
        expire(db, &message, epoch, "targetExpired")?;
    }
    Ok(())
}

pub(super) fn expire_unprovable(db: &Connection, epoch: HostEpoch) -> Result<(), LedgerError> {
    let mut statement = db.prepare("SELECT s.message_id FROM agent_chat_steer_targets s JOIN agent_chat_prompt_dispatches d ON d.message_id=s.message_id WHERE s.state='claimed' AND d.state='unprovable'").map_err(storage_error)?;
    let messages = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    for message in messages {
        expire(db, &message, epoch, "deliveryUnprovable")?;
    }
    Ok(())
}
