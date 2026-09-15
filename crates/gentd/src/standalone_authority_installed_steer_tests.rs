use std::time::{Duration, Instant};

use gent_ports::{AgentChatPromptLedger, AgentChatQueuedPromptLedger, ConversationActivityLedger};
use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatPromptSaved, AgentChatProvider, AgentChatRequestId, ConversationActivityFact,
    DurableTurnPhase, NormalizedTranscriptKind, PROVIDER_SESSION_RECOVERED_NOTICE, ReceiptId,
};

use super::InstalledProvider;
use crate::agent_chat_api::{PromptCommitWake, PromptWake};

fn remembered(
    provider: AgentChatProvider,
) -> (
    InstalledProvider,
    AgentChatConversationId,
    AgentChatPromptSaved,
) {
    let mut installed = InstalledProvider::start(provider);
    let conversation = installed.conversation();
    let seed = installed.send(&conversation, "Remember CODE-HERON");
    assert_eq!(installed.settle(&seed), DurableTurnPhase::Completed);
    (installed, conversation, seed)
}

fn wake_queued(saved: &AgentChatPromptSaved) -> PromptWake {
    PromptWake {
        conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
        run_id: saved.run_id.clone(),
        receipt_id: saved.receipt.receipt_id.clone(),
        disposition: AgentChatPromptDisposition::Queue,
    }
}

impl InstalledProvider {
    fn queue(&mut self, conversation_id: &AgentChatConversationId) -> AgentChatPromptSaved {
        self.prompts += 1;
        let saved = self
            .state
            .ledger()
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("request-{}", self.prompts)),
                receipt_id: ReceiptId(format!("receipt-{}", self.prompts)),
                host_epoch: self.epoch(),
                conversation_id: conversation_id.clone(),
                disposition: AgentChatPromptDisposition::Queue,
                text: "Remember CODE-FALCON".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            })
            .unwrap();
        self.runtime
            .prompt_ingress()
            .wake_after_prompt_commit(wake_queued(&saved))
            .unwrap();
        saved
    }

    fn steer(&self, queued: &AgentChatPromptSaved) {
        let (receipt, _) = self
            .state
            .ledger()
            .steer_queued_agent_chat_prompt(
                &ReceiptId(format!("steer-{}", queued.message.message_id)),
                self.epoch(),
                &AgentChatConversationId(queued.message.conversation_id.clone()),
                &queued.message.message_id,
            )
            .unwrap();
        self.runtime
            .prompt_ingress()
            .steer_run(
                self.provider,
                PromptWake {
                    receipt_id: receipt.receipt_id,
                    ..wake_queued(queued)
                },
            )
            .unwrap();
    }

    fn drive_until_streaming(&self, saved: &AgentChatPromptSaved) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.reply(saved).is_empty() {
            self.runtime.drive_once().unwrap();
            assert!(Instant::now() < deadline, "the turn never streamed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn activity(&self, saved: &AgentChatPromptSaved) -> Vec<ConversationActivityFact> {
        let mut facts = Vec::new();
        let mut after = 0;
        loop {
            let page = self
                .state
                .ledger()
                .read_conversation_activity_page(
                    &saved.message.conversation_id,
                    &saved.run_id.0,
                    after,
                    64,
                )
                .unwrap();
            facts.extend(page.facts);
            match page.next_after_cursor {
                Some(next) => after = next,
                None => return facts,
            }
        }
    }

    fn steered_into_the_running_turn(
        &mut self,
        conversation: &AgentChatConversationId,
    ) -> AgentChatPromptSaved {
        let text = match self.provider {
            AgentChatProvider::Claude => "Remember CODE-OWL then SLOW",
            _ => "Remember CODE-OWL and WAIT",
        };
        let active = self.send(conversation, text);
        self.drive_until_streaming(&active);
        let queued = self.queue(conversation);
        self.steer(&queued);

        assert_eq!(self.settle(&active), DurableTurnPhase::Completed);
        assert_eq!(self.phase(&queued), DurableTurnPhase::Completed);
        let facts = self.activity(&active);
        assert!(
            facts.iter().any(|fact| matches!(fact,
                ConversationActivityFact::PromptSteered { scope, message_id, .. }
                    if *message_id == queued.message.message_id
                        && scope.turn_id == active.message.turn_id)),
            "{facts:?}"
        );
        assert!(!facts.iter().any(|fact| matches!(fact,
            ConversationActivityFact::PromptReleased { message_id, .. }
                if *message_id == queued.message.message_id)));
        let recall = self.send(conversation, "Which code did I give you?");
        assert_eq!(self.settle(&recall), DurableTurnPhase::Completed);
        assert!(self.reply(&recall).contains("CODE-FALCON"));
        active
    }

    fn resumed_launches(&self) -> usize {
        match self.provider {
            AgentChatProvider::Claude => self
                .claude_launches()
                .iter()
                .filter(|argv| argv.iter().any(|flag| flag == "--resume"))
                .count(),
            _ => self.requests("thread/resume").len(),
        }
    }
}

#[test]
fn a_steer_joins_the_running_turn_of_a_session_resumed_after_a_restart() {
    for provider in [AgentChatProvider::Claude, AgentChatProvider::Codex] {
        let (mut installed, conversation, _) = remembered(provider);
        installed.restart();

        installed.steered_into_the_running_turn(&conversation);

        assert_eq!(installed.resumed_launches(), 1, "{provider:?}");
    }
}

#[test]
fn a_steer_joins_the_running_turn_of_a_session_rebound_to_an_upgraded_binary() {
    for provider in [AgentChatProvider::Claude, AgentChatProvider::Codex] {
        let (mut installed, conversation, _) = remembered(provider);
        installed.install("2.0.0");

        installed.steered_into_the_running_turn(&conversation);

        assert_eq!(installed.events("runExecutableRebound").len(), 1);
        assert_eq!(installed.resumed_launches(), 1, "{provider:?}");
    }
}

#[test]
fn a_steer_joins_the_running_turn_claude_recreates_from_saved_history_after_a_restart() {
    let (mut claude, conversation, seed) = remembered(AgentChatProvider::Claude);
    let seed_session = claude.bound_session(&seed);
    claude.restart();
    std::fs::remove_file(
        claude
            .binary()
            .with_file_name("sessions")
            .join(format!("{seed_session}.jsonl")),
    )
    .unwrap();

    let active = claude.steered_into_the_running_turn(&conversation);

    assert_eq!(
        claude.transcript(&active, NormalizedTranscriptKind::Notice),
        [PROVIDER_SESSION_RECOVERED_NOTICE]
    );
    assert_eq!(claude.bound_session(&active), seed_session);
    assert!(claude.claude_launches().iter().any(|argv| {
        argv.windows(2)
            .any(|pair| pair == ["--session-id", seed_session.as_str()])
    }));
}
