//! Pure validation and pagination boundary for agent-chat reads.

use gent_ports::{AgentChatProjectionLedger, AgentChatReadLedger};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationId, AgentChatConversationSummary,
    AgentChatProjectionPage, AgentChatProjectionTail, AgentChatSelection, NormalizedTranscriptPage,
};

use crate::RuntimeError;

/// Read-only runtime service for public agent-chat metadata and transcript pages.
#[derive(Clone, Debug)]
pub struct AgentChatReadService<L> {
    ledger: L,
}

impl<L> AgentChatReadService<L> {
    #[must_use]
    pub fn new(ledger: L) -> Self {
        Self { ledger }
    }
}

impl<L: AgentChatReadLedger> AgentChatReadService<L> {
    /// Reads one provider-neutral conversation summary.
    ///
    /// # Errors
    /// Returns an error when the durable read port cannot resolve the conversation.
    pub fn summary(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationSummary, RuntimeError> {
        Ok(self.ledger.read_agent_chat_summary(conversation_id)?)
    }

    /// Reads one provider-neutral conversation and immutable run hierarchy.
    ///
    /// # Errors
    /// Returns an error when the durable read port fails or returns another conversation.
    pub fn detail(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationDetail, RuntimeError> {
        let detail = self.ledger.read_agent_chat_detail(conversation_id)?;
        if detail.summary.conversation_id != conversation_id {
            return Err(invariant(
                "agent-chat detail belongs to another conversation",
            ));
        }
        Ok(detail)
    }

    /// Resolves the immutable selection fixed on one exact durable run.
    ///
    /// # Errors
    /// Returns an error when the run is absent from the selected conversation.
    pub fn run_selection(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<AgentChatSelection, RuntimeError> {
        self.detail(conversation_id)?
            .runs
            .into_iter()
            .find(|run| run.run_id == run_id)
            .map(|run| run.selection)
            .ok_or_else(|| invariant("agent-chat run is absent from its conversation"))
    }

    /// Reads a bounded ascending transcript page and verifies its cursor invariants.
    ///
    /// # Errors
    /// Returns an error when the durable port fails or violates the public cursor contract.
    pub fn transcript(
        &self,
        conversation_id: &str,
        after_cursor: Option<u64>,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, RuntimeError> {
        let page = self.ledger.read_agent_chat_transcript(
            conversation_id,
            after_cursor,
            limit.clamp(1, 100),
        )?;
        validate_page(conversation_id, after_cursor, &page)?;
        Ok(page)
    }
}

impl<L: AgentChatReadLedger + AgentChatProjectionLedger> AgentChatReadService<L> {
    pub fn projection(
        &self,
        conversation_id: &str,
        after_cursor: u64,
        limit: u16,
    ) -> Result<AgentChatProjectionPage, RuntimeError> {
        let page = self.ledger.agent_chat_projection_page(
            &AgentChatConversationId(conversation_id.to_owned()),
            after_cursor,
            limit.clamp(1, 100),
        )?;
        if page.conversation_id != conversation_id {
            return Err(invariant(
                "agent-chat projection belongs to another conversation",
            ));
        }
        let mut previous = after_cursor;
        for event in &page.events {
            if event.cursor <= previous {
                return Err(invariant(
                    "agent-chat projection cursor is not strictly ascending",
                ));
            }
            previous = event.cursor;
        }
        if let Some(cursor) = page.next_after_cursor {
            if page.events.is_empty() || cursor != previous {
                return Err(invariant(
                    "agent-chat projection continuation does not advance",
                ));
            }
        }
        Ok(page)
    }

    pub fn projection_tail(
        &self,
        conversation_id: &str,
        transcript_limit: u16,
        activity_limit: u16,
    ) -> Result<AgentChatProjectionTail, RuntimeError> {
        let tail = self.ledger.agent_chat_projection_tail(
            &AgentChatConversationId(conversation_id.to_owned()),
            transcript_limit.clamp(1, 100),
            activity_limit.clamp(1, 100),
        )?;
        if tail.conversation_id != conversation_id {
            return Err(invariant(
                "agent-chat projection tail belongs to another conversation",
            ));
        }
        for window in [&tail.transcript, &tail.activity] {
            let mut previous = 0;
            for event in window {
                if event.cursor <= previous || event.cursor > tail.cursor {
                    return Err(invariant(
                        "agent-chat projection tail is not ordered within its cursor",
                    ));
                }
                previous = event.cursor;
            }
        }
        Ok(tail)
    }
}

fn validate_page(
    conversation_id: &str,
    after_cursor: Option<u64>,
    page: &NormalizedTranscriptPage,
) -> Result<(), RuntimeError> {
    if page.conversation_id != conversation_id {
        return Err(invariant(
            "agent-chat transcript belongs to another conversation",
        ));
    }
    let mut previous = after_cursor.unwrap_or(0);
    for event in &page.events {
        if event.cursor <= previous {
            return Err(invariant(
                "agent-chat transcript cursor is not strictly ascending",
            ));
        }
        previous = event.cursor;
    }
    if let Some(cursor) = page.next_after_cursor {
        if page.events.is_empty() || cursor != previous || cursor <= after_cursor.unwrap_or(0) {
            return Err(invariant(
                "agent-chat transcript continuation does not advance",
            ));
        }
    }
    Ok(())
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

#[cfg(test)]
mod tests {
    use super::AgentChatReadService;
    use gent_ports::{AgentChatProjectionLedger, AgentChatReadLedger, LedgerError};
    use gent_types::{
        AgentChatConversationDetail, AgentChatConversationId, AgentChatConversationSummary,
        AgentChatEffort, AgentChatMode, AgentChatProjectionEvent, AgentChatProjectionPage,
        AgentChatProvider, AgentChatSelection, NormalizedTranscriptEvent, NormalizedTranscriptKind,
        NormalizedTranscriptPage,
    };

    #[derive(Clone)]
    struct Ledger {
        page: NormalizedTranscriptPage,
        projection: AgentChatProjectionPage,
        runs: Vec<gent_types::AgentChatRun>,
    }

    impl AgentChatProjectionLedger for Ledger {
        fn agent_chat_projection_page(
            &self,
            _: &AgentChatConversationId,
            _: u64,
            _: u16,
        ) -> Result<AgentChatProjectionPage, LedgerError> {
            Ok(self.projection.clone())
        }

        fn agent_chat_projection_tail(
            &self,
            _: &AgentChatConversationId,
            _: u16,
            _: u16,
        ) -> Result<gent_types::AgentChatProjectionTail, LedgerError> {
            Ok(gent_types::AgentChatProjectionTail {
                conversation_id: self.projection.conversation_id.clone(),
                cursor: self.projection.next_after_cursor.unwrap_or_default(),
                transcript: self.projection.events.clone(),
                activity: Vec::new(),
                transcript_truncated: false,
                activity_truncated: false,
            })
        }
    }

    impl AgentChatReadLedger for Ledger {
        fn read_agent_chat_summary(
            &self,
            _: &str,
        ) -> Result<AgentChatConversationSummary, LedgerError> {
            Ok(summary())
        }
        fn read_agent_chat_detail(
            &self,
            _: &str,
        ) -> Result<AgentChatConversationDetail, LedgerError> {
            Ok(AgentChatConversationDetail {
                summary: summary(),
                current_run_id: "run".into(),
                runs: self.runs.clone(),
            })
        }
        fn read_agent_chat_transcript(
            &self,
            _: &str,
            _: Option<u64>,
            _: u16,
        ) -> Result<NormalizedTranscriptPage, LedgerError> {
            Ok(self.page.clone())
        }
    }

    #[test]
    fn transcript_rejects_a_non_advancing_cursor() {
        let page = page(vec![event(2), event(2)], None);
        assert!(
            AgentChatReadService::new(Ledger {
                page,
                projection: projection(vec![], None),
                runs: vec![]
            })
            .transcript("conversation", Some(1), 20)
            .is_err()
        );
    }

    #[test]
    fn transcript_accepts_the_final_event_cursor_as_the_resume_token() {
        let page = page(vec![event(2), event(3)], Some(3));
        assert_eq!(
            AgentChatReadService::new(Ledger {
                page,
                projection: projection(vec![], None),
                runs: vec![]
            })
            .transcript("conversation", Some(1), 500)
            .unwrap()
            .events
            .len(),
            2
        );
    }

    #[test]
    fn transcript_rejects_a_continuation_that_skips_past_the_final_event() {
        let page = page(vec![event(2), event(3)], Some(4));
        assert!(
            AgentChatReadService::new(Ledger {
                page,
                projection: projection(vec![], None),
                runs: vec![]
            })
            .transcript("conversation", Some(1), 20)
            .is_err()
        );
    }

    #[test]
    fn run_selection_refuses_a_run_from_another_conversation() {
        let ledger = Ledger {
            page: page(vec![], None),
            projection: projection(vec![], None),
            runs: vec![],
        };
        assert!(
            AgentChatReadService::new(ledger)
                .run_selection("conversation", "other-run")
                .is_err()
        );
    }

    #[test]
    fn projection_rejects_a_non_advancing_total_cursor() {
        let ledger = Ledger {
            page: page(vec![], None),
            projection: projection(vec![projection_event(2), projection_event(2)], None),
            runs: vec![],
        };
        assert!(
            AgentChatReadService::new(ledger)
                .projection("conversation", 1, 20)
                .is_err()
        );
    }

    #[test]
    fn projection_uses_its_final_total_cursor_as_the_resume_token() {
        let ledger = Ledger {
            page: page(vec![], None),
            projection: projection(vec![projection_event(2), projection_event(3)], Some(3)),
            runs: vec![],
        };
        assert_eq!(
            AgentChatReadService::new(ledger)
                .projection("conversation", 1, 20)
                .unwrap()
                .events
                .len(),
            2
        );
    }

    #[test]
    fn projection_tail_rejects_a_window_past_its_snapshot_cursor() {
        let ledger = Ledger {
            page: page(vec![], None),
            projection: projection(vec![projection_event(2), projection_event(5)], Some(4)),
            runs: vec![],
        };
        assert!(
            AgentChatReadService::new(ledger)
                .projection_tail("conversation", 20, 20)
                .is_err()
        );
    }

    fn summary() -> AgentChatConversationSummary {
        AgentChatConversationSummary {
            conversation_id: "conversation".into(),
            title: None,
            recap: None,
            workspace_id: None,
            workspace_path: None,
            mcp_server_count: 0,
            mcp_server_names: Vec::new(),
            changed_file_count: None,
            git_branch: None,
            updated_at_unix_ms: 1,
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
        }
    }
    fn event(cursor: u64) -> NormalizedTranscriptEvent {
        NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "ok".into(),
            is_partial: false,
            origin: None,
            attachments: Vec::new(),
        }
    }
    fn page(
        events: Vec<NormalizedTranscriptEvent>,
        next_after_cursor: Option<u64>,
    ) -> NormalizedTranscriptPage {
        NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events,
            next_after_cursor,
        }
    }

    fn projection(
        events: Vec<AgentChatProjectionEvent>,
        next_after_cursor: Option<u64>,
    ) -> AgentChatProjectionPage {
        AgentChatProjectionPage {
            conversation_id: "conversation".into(),
            events,
            next_after_cursor,
        }
    }

    fn projection_event(cursor: u64) -> AgentChatProjectionEvent {
        AgentChatProjectionEvent {
            cursor,
            source_event_id: format!("projection-{cursor}"),
            kind: "transcript".into(),
            payload: serde_json::json!({"text": "ok"}),
        }
    }
}
