use gent_ports::{AgentChatQueuedPromptLedger, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatRejection, AgentChatRunId, ConversationActivityFact,
    ConversationActivityScope, Event, HostEpoch, Receipt, ReceiptId,
};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) use super::agent_chat_queue_commands::{
    queue_command, queued_prompt, receipted, steerable_prompt,
};
use super::queries::{append_event, find_event, storage_error};
use super::{SqliteLedger, conversation_activity_ledger};

pub(super) const STEER_KEY_PREFIX: &str = "agent-chat-steer-queued-prompt:";

impl AgentChatQueuedPromptLedger for SqliteLedger {
    fn steer_queued_agent_chat_prompt_for_turn(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
        message_id: &str,
        run_id: &AgentChatRunId,
        turn_id: &str,
    ) -> Result<(Receipt, AgentChatRunId), LedgerError> {
        super::agent_chat_steer_target::request(
            self,
            receipt_id,
            host_epoch,
            conversation_id,
            message_id,
            run_id,
            turn_id,
        )
    }
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
            if super::agent_chat_stop::cancel_when_returned(
                transaction,
                conversation_id,
                message_id,
                receipt_id,
                host_epoch,
            )? {
                return Ok(());
            }
            let event_id = format!("agent-chat-queue:{message_id}:canceled");
            let (run_id, turn_id, unconfirmed) =
                remove_queued_turn(transaction, conversation_id, message_id, host_epoch)?;
            if unconfirmed && find_event(transaction, &event_id)?.is_some() {
                return Ok(());
            }
            append(
                transaction,
                event_id,
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
    ) -> Result<(Receipt, AgentChatRunId, Option<String>), LedgerError> {
        let command = queue_command(
            receipt_id,
            host_epoch,
            conversation_id,
            message_id,
            format!("{STEER_KEY_PREFIX}{message_id}"),
            "agentChatSteerQueuedPrompt",
        )?;
        let not_steerable = || LedgerError::Rejected(AgentChatRejection::QueuedPromptNotSteerable);
        let mut target = None;
        let receipt = receipted(self, &command, not_steerable(), |transaction| {
            let (run_id, turn_id) = steerable_prompt(transaction, conversation_id, message_id)?
                .ok_or_else(not_steerable)?;
            let live = super::agent_chat_steer_target::live_turn(
                transaction,
                &conversation_id.0,
                None,
                host_epoch,
            )?;
            let Some((live_run, live_turn)) = live else {
                target = Some((run_id.clone(), None));
                return append(
                    transaction,
                    format!("agent-chat-queue:{message_id}:steer-requested"),
                    receipt_id.clone(),
                    ConversationActivityFact::PromptSteerRequested {
                        scope: ConversationActivityScope {
                            conversation_id: conversation_id.0.clone(),
                            run_id,
                            turn_id,
                            host_epoch,
                            cursor: 0,
                        },
                        message_id: message_id.to_owned(),
                        receipt_id: receipt_id.0.clone(),
                    },
                );
            };
            target = Some((live_run.clone(), Some(live_turn.clone())));
            super::agent_chat_steer_target::bind(
                transaction,
                receipt_id,
                host_epoch,
                conversation_id,
                message_id,
                &AgentChatRunId(live_run),
                &live_turn,
            )
        })?;
        let (run_id, turn_id) = match target {
            Some(target) => target,
            None => (
                self.lock()?
                    .query_row(
                        "SELECT run_id FROM conversation_messages WHERE message_id = ?1",
                        [message_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(storage_error)?,
                None,
            ),
        };
        Ok((receipt, AgentChatRunId(run_id), turn_id))
    }

    fn stop_agent_chat_conversation(
        &self,
        command: &gent_types::Command,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Receipt, LedgerError> {
        super::agent_chat_stop::stop(self, command, conversation_id)
    }
}

fn remove_queued_turn(
    transaction: &rusqlite::Transaction<'_>,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
    host_epoch: HostEpoch,
) -> Result<(String, String, bool), LedgerError> {
    let connection: &Connection = transaction;
    let Some((run_id, turn_id)) = queued_prompt(connection, conversation_id, message_id)? else {
        return connection
            .query_row(
                "SELECT m.run_id, m.turn_id FROM conversation_messages m
                 JOIN agent_chat_prompt_dispatches d ON d.message_id = m.message_id
                 JOIN agent_chat_prompt_receipts p ON p.message_id = m.message_id
                 JOIN agent_chat_steer_targets s ON s.message_id = m.message_id
                 JOIN turns t ON t.turn_id = m.turn_id
                 WHERE m.message_id = ?1 AND m.conversation_id = ?2 AND p.disposition = 'queue'
                   AND d.state = 'unprovable' AND s.state = 'rejected' AND t.phase = 'failed'",
                params![message_id, conversation_id.0],
                |row| Ok((row.get(0)?, row.get(1)?, true)),
            )
            .optional()
            .map_err(storage_error)?
            .ok_or(LedgerError::Rejected(
                AgentChatRejection::QueuedPromptNotCancelable,
            ));
    };
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
    Ok((run_id, turn_id, false))
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
