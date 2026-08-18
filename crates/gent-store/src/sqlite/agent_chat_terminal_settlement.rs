//! `SQLite` read path for one durably settled provider-bound agent-chat turn.

use gent_ports::{LedgerError, agent_chat_terminal_settlement::AgentChatTerminalSettlementReader};
use gent_types::{AgentChatTerminalSettlement, DurableTurnPhase};
use rusqlite::{OptionalExtension, params};

use super::{
    SqliteLedger,
    queries::{host_ingress, storage_error},
};

impl AgentChatTerminalSettlementReader for SqliteLedger {
    fn read_agent_chat_terminal_settlement(
        &self,
        run_id: &str,
        turn_id: &str,
    ) -> Result<Option<AgentChatTerminalSettlement>, LedgerError> {
        if run_id.trim().is_empty() || turn_id.trim().is_empty() {
            return Err(LedgerError::Invariant(
                "terminal settlement identity is invalid".into(),
            ));
        }
        let connection = self.lock()?;
        let host_epoch = host_ingress(&connection)?.epoch;
        connection
            .query_row(
                "SELECT m.conversation_id, m.run_id, m.turn_id, t.phase, COALESCE(MAX(e.cursor), 0) FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN turns t ON t.turn_id = m.turn_id LEFT JOIN agent_chat_transcript_events e ON e.conversation_id = m.conversation_id AND e.run_id = m.run_id AND e.turn_id = m.turn_id WHERE d.state = 'settled' AND m.run_id = ?1 AND m.turn_id = ?2 AND t.phase IN ('completed', 'interrupted', 'failed') GROUP BY m.conversation_id, m.run_id, m.turn_id, t.phase",
                params![run_id, turn_id],
                |row| Ok(AgentChatTerminalSettlement {
                    conversation_id: row.get(0)?,
                    run_id: row.get(1)?,
                    turn_id: row.get(2)?,
                    host_epoch,
                    cursor: row.get(4)?,
                    phase: phase(&row.get::<_, String>(3)?)?,
                }),
            )
            .optional()
            .map_err(storage_error)
    }
}

fn phase(value: &str) -> rusqlite::Result<DurableTurnPhase> {
    match value {
        "completed" => Ok(DurableTurnPhase::Completed),
        "interrupted" => Ok(DurableTurnPhase::Interrupted),
        "failed" => Ok(DurableTurnPhase::Failed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
