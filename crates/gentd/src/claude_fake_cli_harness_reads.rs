use std::fs;

use gent_ports::{AgentChatProjectionLedger, AgentChatReadLedger, ConversationLedger, Ledger};
use gent_types::{
    AgentChatConversationId, AgentChatProjectionEvent, AgentChatPromptSaved, AgentChatRunId,
    DurableTurnPhase, NormalizedTranscriptEvent,
};

use super::FakeClaudeDaemon;

impl FakeClaudeDaemon {
    pub(crate) fn phase(&self, prompt: &AgentChatPromptSaved) -> DurableTurnPhase {
        self.ledger
            .find_turn(&prompt.message.turn_id)
            .unwrap()
            .unwrap()
            .phase
    }

    pub(crate) fn transcript(
        &self,
        prompt: &AgentChatPromptSaved,
    ) -> Vec<NormalizedTranscriptEvent> {
        let mut events = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .ledger
                .read_agent_chat_transcript(&prompt.message.conversation_id, after, 100)
                .unwrap();
            events.extend(
                page.events
                    .into_iter()
                    .filter(|event| event.turn_id == prompt.message.turn_id),
            );
            match page.next_after_cursor {
                Some(next) => after = Some(next),
                None => return events,
            }
        }
    }

    pub(crate) fn projection(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Vec<AgentChatProjectionEvent> {
        let mut events = Vec::new();
        loop {
            let after = events
                .last()
                .map_or(0, |event: &AgentChatProjectionEvent| event.cursor);
            let page = self
                .ledger
                .agent_chat_projection_page(conversation_id, after, 100)
                .unwrap();
            events.extend(page.events);
            if page.next_after_cursor.is_none() {
                return events;
            }
        }
    }

    pub(crate) fn launches(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.root.path().join("cli/launches.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Vec<String>>(line).unwrap())
            .filter(|argv| {
                !argv
                    .iter()
                    .any(|argument| argument == "--no-session-persistence")
            })
            .collect()
    }

    pub(crate) fn sessions(&self) -> Vec<String> {
        fs::read_dir(self.root.path().join("cli/sessions"))
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    pub(crate) fn bound_session(&self, run_id: &AgentChatRunId) -> String {
        self.ledger
            .find_run_session_binding(&run_id.0)
            .unwrap()
            .unwrap()
            .provider_session_id
    }

    pub(crate) fn session_prompts(&self, session_id: &str) -> usize {
        fs::read_to_string(
            self.root
                .path()
                .join(format!("cli/sessions/{session_id}.jsonl")),
        )
        .unwrap()
        .lines()
        .count()
    }
}
