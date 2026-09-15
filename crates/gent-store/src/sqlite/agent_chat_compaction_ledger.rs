//! Indexed canonical-event read for one run's normalized compaction facts.

use gent_ports::{AgentChatCompactionLedger, ContextCompactionLedger, LedgerError};
use gent_types::{
    AgentChatConversationId, CONTEXT_COMPACTION_EVENT_KIND, ContextCompactionFact, Event,
    EventPage, HostEpoch, NormalizedTranscriptAppend, NormalizedTranscriptKind, ReceiptId,
};
use rusqlite::{TransactionBehavior, params};

use super::{
    SqliteLedger, epoch::require_epoch, event_pages, queries, transcript_ledger::append_in,
};

impl AgentChatCompactionLedger for SqliteLedger {
    fn read_agent_chat_compaction_page(
        &self,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<EventPage, LedgerError> {
        event_pages::read_compaction(&*self.lock()?, run_id, after_cursor, limit)
    }
}

impl ContextCompactionLedger for SqliteLedger {
    fn record_context_compaction(
        &self,
        fact: &ContextCompactionFact,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        if !fact.is_valid() {
            return Err(LedgerError::Invariant(
                "context compaction fact is invalid".into(),
            ));
        }
        let payload = serde_json::to_value(fact).map_err(queries::storage_error)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(queries::storage_error)?;
        require_epoch(host_epoch, queries::host_ingress(&transaction)?.epoch)?;
        match queries::find_event(&transaction, &fact.event_id())? {
            Some(existing) if existing.payload == payload => return Ok(()),
            Some(_) => {
                return Err(LedgerError::Invariant(
                    "context compaction retry conflicts with its recorded fact".into(),
                ));
            }
            None => {}
        }
        queries::append_event(
            &transaction,
            &Event {
                cursor: 0,
                event_id: fact.event_id(),
                receipt_id: ReceiptId(fact.event_id()),
                host_epoch,
                kind: CONTEXT_COMPACTION_EVENT_KIND.into(),
                payload,
            },
        )?;
        append_in(
            &transaction,
            &AgentChatConversationId(fact.conversation_id().into()),
            &NormalizedTranscriptAppend {
                event_id: fact.notice_event_id(),
                turn_id: fact.turn_id().into(),
                run_id: fact.run_id().into(),
                kind: NormalizedTranscriptKind::Notice,
                text: fact.notice().into(),
                is_partial: false,
            },
        )?;
        transaction.commit().map_err(queries::storage_error)
    }

    fn context_compactions(
        &self,
        conversation_id: &str,
        limit: u16,
    ) -> Result<Vec<ContextCompactionFact>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT payload FROM events WHERE kind = ?1 \
                 AND json_extract(payload, '$.conversationId') = ?2 \
                 ORDER BY cursor DESC LIMIT ?3",
            )
            .map_err(queries::storage_error)?;
        let payloads = statement
            .query_map(
                params![CONTEXT_COMPACTION_EVENT_KIND, conversation_id, limit.max(1)],
                |row| row.get::<_, String>(0),
            )
            .map_err(queries::storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(queries::storage_error)?;
        payloads
            .iter()
            .map(|payload| {
                serde_json::from_str(payload).map_err(|_| {
                    LedgerError::Storage("context compaction fact is unreadable".into())
                })
            })
            .collect()
    }
}
