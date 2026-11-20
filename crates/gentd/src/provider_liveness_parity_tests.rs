use std::time::{Duration, Instant};

use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::AgentChatPromptLedger;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatRequestId,
    HostEpoch, NormalizedLifecycleSignal, ReceiptId, TurnPhase,
};

use super::claude_warm_pool_tests::Pool as ClaudePool;
use super::codex_warm_pool_tests::Pool as CodexPool;
use crate::provider_warm_pool::WarmPoolRequest;

const SERVED_IDLE: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Situation {
    TurnOpen,
    SettledWithBackgroundWork,
    SettledIdle,
    Releasing,
}

#[derive(Debug, Eq, PartialEq)]
struct Answer {
    polls: bool,
    reaped: bool,
    accepts_prompt: bool,
}

trait Provider {
    fn new() -> Self;
    fn ledger(&self) -> &SqliteLedger;
    fn conversation(&self) -> AgentChatConversationId;
    fn open_turn(&mut self, conversation: &AgentChatConversationId) -> String;
    fn settle(&mut self, run_id: &str);
    fn report_session(&mut self, run_id: &str, idle: bool);
    fn release(&mut self);
    fn polls(&self) -> bool;
    fn reap(&mut self, now: Instant);
    fn released(&self) -> bool;
    fn starts_queued_prompt(&mut self) -> bool;

    fn arrange(situation: Situation) -> (Self, AgentChatConversationId)
    where
        Self: Sized,
    {
        let mut provider = Self::new();
        let conversation = provider.conversation();
        let run_id = provider.open_turn(&conversation);
        match situation {
            Situation::TurnOpen => {}
            Situation::SettledWithBackgroundWork => {
                provider.report_session(&run_id, false);
                provider.settle(&run_id);
            }
            Situation::SettledIdle => {
                provider.settle(&run_id);
                provider.report_session(&run_id, true);
            }
            Situation::Releasing => {
                provider.settle(&run_id);
                provider.release();
            }
        }
        (provider, conversation)
    }

    fn answer(situation: Situation) -> Answer
    where
        Self: Sized,
    {
        let (mut observed, _) = Self::arrange(situation);
        let polls = observed.polls();
        let now = Instant::now();
        observed.reap(now);
        observed.reap(now + SERVED_IDLE);
        let reaped = observed.released();
        let (mut queued, conversation) = Self::arrange(situation);
        queue(queued.ledger(), &conversation);
        Answer {
            polls,
            reaped,
            accepts_prompt: queued.starts_queued_prompt(),
        }
    }
}

fn queue(ledger: &SqliteLedger, conversation: &AgentChatConversationId) {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("queued".into()),
            receipt_id: ReceiptId("queued-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "queued".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
}

impl Provider for ClaudePool {
    fn new() -> Self {
        ClaudePool::new(4)
    }
    fn ledger(&self) -> &SqliteLedger {
        &self.ledger
    }
    fn conversation(&self) -> AgentChatConversationId {
        ClaudePool::conversation(self, "a")
    }
    fn open_turn(&mut self, conversation: &AgentChatConversationId) -> String {
        self.send(conversation, "first")
    }
    fn settle(&mut self, run_id: &str) {
        self.finish(run_id.into());
    }
    fn report_session(&mut self, run_id: &str, idle: bool) {
        self.runner
            .0
            .lock()
            .unwrap()
            .effects
            .entry(run_id.into())
            .or_default()
            .push_back(vec![ClaudeRunnerEffect::SessionState { idle }]);
        self.host.tick().unwrap();
    }
    fn release(&mut self) {
        self.host
            .signal_active(ProcessTreeSignal::Terminate)
            .unwrap();
    }
    fn polls(&self) -> bool {
        self.host.needs_drive()
    }
    fn reap(&mut self, now: Instant) {
        self.host
            .tend_warm_pool(&WarmPoolRequest { now, target: None })
            .unwrap();
    }
    fn released(&self) -> bool {
        !self.runner.0.lock().unwrap().releases.is_empty()
    }
    fn starts_queued_prompt(&mut self) -> bool {
        matches!(
            self.host.tick().unwrap().dispatch,
            Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
        )
    }
}

impl Provider for CodexPool {
    fn new() -> Self {
        CodexPool::new(4)
    }
    fn ledger(&self) -> &SqliteLedger {
        &self.ledger
    }
    fn conversation(&self) -> AgentChatConversationId {
        CodexPool::conversation(self, "a")
    }
    fn open_turn(&mut self, conversation: &AgentChatConversationId) -> String {
        self.send(conversation, "first")
    }
    fn settle(&mut self, run_id: &str) {
        self.finish(run_id);
    }
    fn report_session(&mut self, run_id: &str, idle: bool) {
        self.runner
            .state
            .lock()
            .unwrap()
            .run_effects
            .entry(run_id.into())
            .or_default()
            .push_back(vec![CodexRunnerEffect::SessionState { idle }]);
        self.host.tick().unwrap();
    }
    fn release(&mut self) {
        let run_id = self.open_turn(&CodexPool::conversation(self, "a"));
        self.runner
            .state
            .lock()
            .unwrap()
            .run_effects
            .entry(run_id)
            .or_default()
            .push_back(vec![CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Failed,
                },
            ))]);
        self.host.tick().unwrap();
    }
    fn polls(&self) -> bool {
        self.host.needs_drive()
    }
    fn reap(&mut self, now: Instant) {
        self.host
            .tend_warm_pool(&WarmPoolRequest { now, target: None })
            .unwrap();
    }
    fn released(&self) -> bool {
        !self.runner.state.lock().unwrap().releases.is_empty()
    }
    fn starts_queued_prompt(&mut self) -> bool {
        matches!(
            self.host.tick().unwrap().dispatch,
            Some(crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome::Started { .. })
        )
    }
}

fn same_answer(situation: Situation, expected: &Answer) {
    let claude = ClaudePool::answer(situation);
    let codex = CodexPool::answer(situation);
    assert_eq!(claude, codex, "{situation:?}");
    assert_eq!(&claude, expected, "{situation:?}");
}

#[test]
fn an_open_turn_is_polled_kept_and_blocks_the_next_prompt_for_both_providers() {
    same_answer(
        Situation::TurnOpen,
        &Answer {
            polls: true,
            reaped: false,
            accepts_prompt: false,
        },
    );
}

#[test]
fn a_settled_turn_with_background_work_is_kept_but_accepts_the_next_prompt_for_both_providers() {
    same_answer(
        Situation::SettledWithBackgroundWork,
        &Answer {
            polls: true,
            reaped: false,
            accepts_prompt: true,
        },
    );
}

#[test]
fn a_settled_idle_session_is_reaped_after_the_served_timeout_for_both_providers() {
    same_answer(
        Situation::SettledIdle,
        &Answer {
            polls: false,
            reaped: true,
            accepts_prompt: true,
        },
    );
}

#[test]
fn a_releasing_session_is_polled_until_exit_and_takes_no_prompt_for_both_providers() {
    same_answer(
        Situation::Releasing,
        &Answer {
            polls: true,
            reaped: false,
            accepts_prompt: false,
        },
    );
}
