use gent_ports::{ConversationLedger, TranscriptLedger};
use gent_protocol::{AgentChatIntentFrame, HistoricalTranscriptEntry};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, DurableTurnPhase,
    NormalizedTranscriptAppend, NormalizedTranscriptKind, TurnRecord,
};

use super::RuntimeFacade;

impl RuntimeFacade {
    pub(super) fn import_transcript(
        &self,
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        entries: &[HistoricalTranscriptEntry],
    ) -> Result<AgentChatIntentFrame, String> {
        let imported_count = u16::try_from(entries.len())
            .ok()
            .filter(|count| *count <= 500)
            .ok_or("historical transcript import exceeds 500 entries")?;
        for entry in entries {
            if entry.source_id.trim().is_empty()
                || entry.source_id.len() > 512
                || entry.source_id.contains('\0')
                || entry.text.len() > 65_536
                || !matches!(
                    entry.kind,
                    NormalizedTranscriptKind::UserMessage
                        | NormalizedTranscriptKind::AssistantMessage
                )
            {
                return Err("historical transcript entry is invalid".into());
            }
        }
        let import_turn_id = format!("historical-import:{}", run_id.0);
        let run_turns = self
            .transcript_import_ledger
            .list_run_turns(&run_id.0)
            .map_err(|error| error.to_string())?;
        if !run_turns.iter().any(|turn| turn.turn_id == import_turn_id) {
            if !run_turns.is_empty() {
                return Err("historical transcript import must precede the first prompt".into());
            }
            self.transcript_import_ledger
                .create_turn(&TurnRecord {
                    turn_id: import_turn_id.clone(),
                    conversation_id: conversation_id.0.clone(),
                    run_id: run_id.0.clone(),
                    sequence: 1,
                    phase: DurableTurnPhase::Active,
                })
                .map_err(|error| error.to_string())?;
            self.transcript_import_ledger
                .replace_turn_phase(
                    &import_turn_id,
                    DurableTurnPhase::Active,
                    DurableTurnPhase::Completed,
                )
                .map_err(|error| error.to_string())?;
        }
        for entry in entries {
            self.transcript_import_ledger
                .append_normalized_transcript(
                    &conversation_id,
                    &NormalizedTranscriptAppend {
                        event_id: format!("import:{}", entry.source_id),
                        turn_id: import_turn_id.clone(),
                        run_id: run_id.0.clone(),
                        kind: entry.kind,
                        text: entry.text.clone(),
                        is_partial: false,
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(AgentChatIntentFrame::TranscriptImported {
            request_id,
            conversation_id,
            run_id,
            imported_count,
        })
    }
}
