use gent_core::activity_scope;
use gent_ports::{ConversationLinkLedger, LedgerError};
use gent_types::{
    ConversationActivityFact, ConversationLink, ConversationThreadState, Event, ReceiptId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::queries::{append_event, find_event, storage_error};
use super::{SqliteLedger, conversation_activity_ledger};

const COLUMNS: &str = "SELECT parent_conversation_id, child_conversation_id, label, created_run_id, created_turn_id, created_at_unix_seconds FROM agent_chat_conversation_links";

impl ConversationLinkLedger for SqliteLedger {
    fn record_conversation_link(
        &self,
        link: &ConversationLink,
        facts: &[ConversationActivityFact],
        request_identity: &str,
    ) -> Result<(), LedgerError> {
        validate(link)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        if let Some(existing) = read_parent(&transaction, &link.child_conversation_id)? {
            if existing.parent_conversation_id != link.parent_conversation_id {
                return Err(LedgerError::Invariant(
                    "conversation already belongs to another parent".into(),
                ));
            }
        } else {
            transaction
                .execute(
                    "INSERT INTO agent_chat_conversation_links (child_conversation_id, parent_conversation_id, label, created_run_id, created_turn_id, created_at_unix_seconds) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        link.child_conversation_id,
                        link.parent_conversation_id,
                        link.label,
                        link.created_run_id,
                        link.created_turn_id,
                        link.created_at_unix_seconds,
                    ],
                )
                .map_err(storage_error)?;
        }
        for (ordinal, fact) in facts.iter().enumerate() {
            publish(&transaction, fact, &format!("{request_identity}:{ordinal}"))?;
        }
        transaction.commit().map_err(storage_error)
    }

    fn append_conversation_orchestration_fact(
        &self,
        fact: &ConversationActivityFact,
        request_identity: &str,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        publish(&transaction, fact, request_identity)?;
        transaction.commit().map_err(storage_error)
    }

    fn read_conversation_thread_state(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationThreadState>, LedgerError> {
        thread_state(&*self.lock()?, conversation_id)
    }

    fn read_conversation_parent(
        &self,
        child_conversation_id: &str,
    ) -> Result<Option<ConversationLink>, LedgerError> {
        read_parent(&*self.lock()?, child_conversation_id)
    }

    fn read_conversation_children(
        &self,
        parent_conversation_id: &str,
    ) -> Result<Vec<ConversationLink>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "{COLUMNS} WHERE parent_conversation_id = ?1 ORDER BY created_at_unix_seconds ASC, child_conversation_id ASC LIMIT 256"
            ))
            .map_err(storage_error)?;
        let rows = statement
            .query_map([parent_conversation_id], decode)
            .map_err(storage_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage_error)
    }
}

fn validate(link: &ConversationLink) -> Result<(), LedgerError> {
    if [
        &link.parent_conversation_id,
        &link.child_conversation_id,
        &link.label,
        &link.created_run_id,
        &link.created_turn_id,
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
        || link.parent_conversation_id == link.child_conversation_id
    {
        return Err(LedgerError::Invariant(
            "a conversation link needs distinct nonempty identities and a label".into(),
        ));
    }
    Ok(())
}

fn publish(
    transaction: &Transaction<'_>,
    fact: &ConversationActivityFact,
    request_identity: &str,
) -> Result<(), LedgerError> {
    let scope = activity_scope(fact);
    let identity = format!("conversation-link:{request_identity}");
    if find_event(transaction, &identity)?.is_some() {
        return Ok(());
    }
    let event = append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: identity.clone(),
            receipt_id: ReceiptId(identity),
            host_epoch: scope.host_epoch,
            kind: "agentChatConversationLinkActivity".into(),
            payload: serde_json::json!({
                "conversationId": scope.conversation_id,
                "runId": scope.run_id,
                "turnId": scope.turn_id,
                "activity": fact,
            }),
        },
    )?;
    let assigned = gent_core::with_activity_cursor(fact.clone(), event.cursor);
    conversation_activity_ledger::append(transaction, &assigned)?;
    let scope = activity_scope(&assigned);
    transaction
        .execute(
            "INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES (?1, ?2, 'activity', ?3)",
            params![
                scope.conversation_id,
                format!("activity:{}", event.event_id),
                serde_json::json!({
                    "conversationId": scope.conversation_id,
                    "runId": scope.run_id,
                    "turnId": scope.turn_id,
                    "activity": assigned,
                })
                .to_string(),
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn thread_state(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Option<ConversationThreadState>, LedgerError> {
    let Some((root_run_id, workspace_id, updated_at_unix_ms)) = connection
        .query_row(
            "SELECT root_run_id, workspace_id, updated_at_unix_ms FROM agent_chat_conversations WHERE conversation_id = ?1",
            [conversation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?
    else {
        return Ok(None);
    };
    let latest = connection
        .query_row(
            "SELECT turn_id, run_id, phase FROM turns WHERE conversation_id = ?1 ORDER BY rowid DESC LIMIT 1",
            [conversation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    super::conversations::decode_phase(&row.get::<_, String>(2)?)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let last_assistant_text = connection
        .query_row(
            "SELECT text FROM agent_chat_transcript_events WHERE conversation_id = ?1 AND kind = 'assistantMessage' AND is_partial = 0 ORDER BY cursor DESC LIMIT 1",
            [conversation_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    Ok(Some(ConversationThreadState {
        conversation_id: conversation_id.to_owned(),
        run_id: latest
            .as_ref()
            .map_or(root_run_id, |(_, run_id, _)| run_id.clone()),
        turn_id: latest.as_ref().map(|(turn_id, _, _)| turn_id.clone()),
        workspace_id,
        phase: latest.map(|(_, _, phase)| phase),
        last_assistant_text,
        last_activity_unix_seconds: updated_at_unix_ms / 1000,
    }))
}

fn read_parent(
    connection: &Connection,
    child_conversation_id: &str,
) -> Result<Option<ConversationLink>, LedgerError> {
    connection
        .query_row(
            &format!("{COLUMNS} WHERE child_conversation_id = ?1"),
            [child_conversation_id],
            decode,
        )
        .optional()
        .map_err(storage_error)
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationLink> {
    Ok(ConversationLink {
        parent_conversation_id: row.get(0)?,
        child_conversation_id: row.get(1)?,
        label: row.get(2)?,
        created_run_id: row.get(3)?,
        created_turn_id: row.get(4)?,
        created_at_unix_seconds: row.get(5)?,
    })
}
