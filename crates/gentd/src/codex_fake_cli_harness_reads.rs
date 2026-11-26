use std::fs;

use gent_ports::{AgentChatProjectionLedger, AgentChatReadLedger, ConversationLedger, Ledger};
use gent_types::{
    AgentChatConversationId, AgentChatProjectionEvent, AgentChatPromptSaved, AgentChatRunId,
    DurableTurnPhase, NormalizedTranscriptEvent, NormalizedTranscriptKind,
};
use serde_json::Value;

use super::FakeCodexDaemon;

impl FakeCodexDaemon {
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

    pub(crate) fn reply(&self, prompt: &AgentChatPromptSaved) -> String {
        self.transcript(prompt)
            .into_iter()
            .filter(|event| event.kind == NormalizedTranscriptKind::AssistantMessage)
            .map(|event| event.text)
            .collect()
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

    pub(crate) fn requests(&self, method: &str) -> Vec<Value> {
        let requests = fs::read_to_string(self.root.path().join("cli/requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let workspace = self.workspace.display().to_string();
        let conversation_launches = requests
            .iter()
            .filter(|request| {
                matches!(
                    request["method"].as_str(),
                    Some("thread/start" | "thread/resume")
                ) && request["params"]["cwd"] == workspace.as_str()
            })
            .map(|request| request["launch"].clone())
            .collect::<Vec<_>>();
        requests
            .into_iter()
            .filter(|request| {
                request["method"] == method && conversation_launches.contains(&request["launch"])
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
}
