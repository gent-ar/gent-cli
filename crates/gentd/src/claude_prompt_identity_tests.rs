use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatReadLedger, AgentChatWorkspaceLedger, ConversationActivityLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet,
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, ReceiptId, TurnPhase,
    WorkspaceRecord,
};

use super::{Resolver, Runner, compatibility, profile, prompt};
use crate::approved_claude_host::ApprovedClaudeHost;
use crate::public_driver_runtime::PublicDriversRuntime;

struct Session {
    ledger: SqliteLedger,
    runner: Runner,
    host: ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
    conversation: AgentChatConversationId,
}

impl Session {
    fn answered_first_prompt() -> Self {
        let ledger = SqliteLedger::in_memory().unwrap();
        let conversation = AgentChatConversationId("conversation".into());
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("conversation-receipt".into()),
                    idempotency_key: "conversation-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation.clone(),
                    run_id: AgentChatRunId("run".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claude,
                        model: "claude-test".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace".into(),
                    canonical_path: "/workspace".into(),
                },
            )
            .unwrap();
        let runner = Runner::default();
        let compatibility = compatibility();
        let runtime = PublicDriversRuntime::new(
            profile(&compatibility),
            Coordinator::new(ledger.clone(), CapabilitySet::default()),
            ledger.clone(),
            compatibility,
            runner.clone(),
            Resolver,
        )
        .unwrap();
        let host = ApprovedClaudeHost::new(runtime, "daemon".into(), HostEpoch(1), 1, None);
        let mut session = Self {
            ledger,
            runner,
            host,
            conversation,
        };
        let first = prompt(&session.ledger, &session.conversation, "first");
        session.host.tick().unwrap();
        session.provider(vec![
            started(&first),
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: "session".into(),
            }),
            output("launched"),
            result(&[&first], false),
            ready(),
        ]);
        assert_eq!(
            session.terminal(&first.message.turn_id),
            Some("ready".into())
        );
        session
    }

    fn provider(&mut self, effects: Vec<ClaudeRunnerEffect>) {
        self.runner.0.lock().unwrap().effects.push_back(effects);
        self.host.tick().unwrap();
    }

    fn write_prompt(&mut self, key: &str) -> AgentChatPromptSaved {
        let saved = prompt(&self.ledger, &self.conversation, key);
        self.host.tick().unwrap();
        saved
    }

    fn terminal(&self, turn_id: &str) -> Option<String> {
        let phases = self
            .ledger
            .read_conversation_activity_page(&self.conversation.0, "run", 0, 50)
            .unwrap()
            .facts
            .iter()
            .map(|fact| serde_json::to_value(fact).unwrap())
            .filter(|fact| fact["type"] == "terminal" && fact["turnId"] == turn_id)
            .map(|fact| fact["phase"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert!(
            phases.len() <= 1,
            "turn {turn_id} settled twice: {phases:?}"
        );
        phases.into_iter().next()
    }

    fn turn_of(&self, text: &str) -> String {
        self.ledger
            .read_agent_chat_transcript(&self.conversation.0, None, 100)
            .unwrap()
            .events
            .into_iter()
            .find(|event| event.text == text && !event.is_partial)
            .unwrap()
            .turn_id
    }
}

fn started(prompt: &AgentChatPromptSaved) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::MessageStarted {
        message_id: prompt.message.message_id.clone(),
    }
}

fn result(prompts: &[&AgentChatPromptSaved], continuation: bool) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::TurnResult {
        message_ids: prompts
            .iter()
            .map(|prompt| prompt.message.message_id.clone())
            .collect(),
        continuation,
    }
}

fn output(text: &str) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
        text: text.into(),
        is_partial: false,
    }))
}

fn ready() -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready,
        },
    ))
}

fn continuation_started() -> Vec<ClaudeRunnerEffect> {
    vec![
        ClaudeRunnerEffect::ContinuationStarted,
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "session".into(),
        }),
        output("background agent finished"),
    ]
}

#[test]
fn a_prompt_queued_behind_a_continuation_is_not_settled_by_the_continuation_result() {
    let mut session = Session::answered_first_prompt();
    let queued = session.write_prompt("queued");
    let mut continuation = continuation_started();
    continuation.extend([result(&[], true), ready()]);
    session.provider(continuation);
    let continuation_turn = session.turn_of("background agent finished");
    assert_ne!(continuation_turn, queued.message.turn_id);
    assert_eq!(session.terminal(&continuation_turn), Some("ready".into()));
    assert_eq!(session.terminal(&queued.message.turn_id), None);

    session.provider(vec![
        started(&queued),
        output("HELLO"),
        result(&[&queued], false),
        ready(),
    ]);
    assert_eq!(session.turn_of("HELLO"), queued.message.turn_id);
    assert_eq!(
        session.terminal(&queued.message.turn_id),
        Some("ready".into())
    );
    assert!(!session.host.needs_drive());
}

#[test]
fn a_prompt_taken_up_inside_a_continuation_is_answered_and_settled_on_its_own_turn() {
    let mut session = Session::answered_first_prompt();
    let queued = session.write_prompt("queued");
    let mut continuation = continuation_started();
    continuation.extend([
        started(&queued),
        output("HELLO"),
        result(&[&queued], true),
        ready(),
    ]);
    session.provider(continuation);
    let continuation_turn = session.turn_of("background agent finished");
    assert_ne!(continuation_turn, queued.message.turn_id);
    assert_eq!(session.terminal(&continuation_turn), Some("ready".into()));
    assert_eq!(session.turn_of("HELLO"), queued.message.turn_id);
    assert_eq!(
        session.terminal(&queued.message.turn_id),
        Some("ready".into())
    );
    assert!(!session.host.needs_drive());
}
