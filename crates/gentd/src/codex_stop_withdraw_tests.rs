use super::*;
use gent_drivers::codex_session::CodexSteerOutcome;
use gent_ports::AgentChatQueuedPromptLedger;
use gent_types::{AgentChatPromptSaved, Command};

struct Running {
    ledger: SqliteLedger,
    runner: Runner,
    host: ApprovedCodexHost<SqliteLedger, Runner, Resolver>,
    conversation: AgentChatConversationId,
    turn: String,
}

fn queue(running: &Running, key: &str) -> AgentChatPromptSaved {
    let saved = running
        .ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(key.into()),
            receipt_id: ReceiptId(format!("{key}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id: running.conversation.clone(),
            disposition: AgentChatPromptDisposition::Queue,
            text: key.into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(&running.ledger, &saved);
    saved
}

fn running() -> Running {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = setup_conversation(&ledger);
    save(&ledger, &conversation, "active", "first");
    let turn = ledger.list_run_turns("run-a").unwrap()[0].turn_id.clone();
    let runner = Runner::default();
    let mut host = host(&ledger, runner.clone());
    host.tick().unwrap();
    runner
        .state
        .lock()
        .unwrap()
        .effects
        .push_back(vec![CodexRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "native-thread".into(),
            },
        )]);
    host.tick().unwrap();
    Running {
        ledger,
        runner,
        host,
        conversation,
        turn,
    }
}

fn facts(running: &Running) -> Vec<ConversationActivityFact> {
    running
        .ledger
        .read_conversation_activity_page(&running.conversation.0, "run-a", 0, 50)
        .unwrap()
        .facts
}

fn withdrawn(running: &Running) -> Vec<String> {
    facts(running)
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptWithdrawn { message_id, .. } => Some(message_id),
            _ => None,
        })
        .collect()
}

fn interrupted(running: &mut Running, unconfirmed: &[&AgentChatPromptSaved]) {
    running.host.interrupt_turn("run-a", &running.turn).unwrap();
    let mut effects = unconfirmed
        .iter()
        .map(|prompt| {
            CodexRunnerEffect::Steer(CodexSteerOutcome::Unconfirmed {
                message_id: prompt.message.message_id.clone(),
            })
        })
        .collect::<Vec<_>>();
    effects.push(CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Interrupted,
        },
    )));
    let mut state = running.runner.state.lock().unwrap();
    state.effects.push_back(effects);
    state
        .effects
        .push_back(vec![CodexRunnerEffect::Exited { code: Some(0) }]);
    drop(state);
    for _ in 0..4 {
        running.host.tick().unwrap();
    }
}

#[test]
fn stop_withdraws_held_and_written_codex_prompts_and_runs_nothing_after() {
    let mut running = running();
    let written = queue(&running, "written");
    running.host.tick().unwrap();
    assert_eq!(running.runner.state.lock().unwrap().steers.len(), 1);
    let held = queue(&running, "held");
    running
        .ledger
        .stop_agent_chat_conversation(
            &Command {
                receipt_id: ReceiptId("stop".into()),
                idempotency_key: "agent-chat-interrupt:stop".into(),
                host_epoch: HostEpoch(1),
                kind: "agentChatInterrupt".into(),
                payload: serde_json::json!({"conversationId": "conversation-a"}),
            },
            &running.conversation,
        )
        .unwrap();
    assert_eq!(
        withdrawn(&running),
        std::slice::from_ref(&held.message.message_id)
    );
    interrupted(&mut running, &[&written]);
    assert_eq!(
        withdrawn(&running),
        [
            held.message.message_id.clone(),
            written.message.message_id.clone()
        ]
    );
    assert!(!facts(&running).iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::PromptReleased { .. }
            | ConversationActivityFact::Terminal {
                cause: Some(gent_types::TurnTerminalCause::Steered),
                ..
            }
    )));
    let state = running.runner.state.lock().unwrap();
    assert!(state.submitted.is_empty());
    assert_eq!((state.starts, state.steers.len()), (1, 1));
}

#[test]
fn send_now_interrupts_codex_and_runs_the_pressed_prompt_before_the_returned_one() {
    let mut running = running();
    let written = queue(&running, "written");
    running.host.tick().unwrap();
    let pressed = queue(&running, "pressed");
    let (_, _, turn) = running
        .ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("send-now".into()),
            HostEpoch(1),
            &running.conversation,
            &pressed.message.message_id,
        )
        .unwrap();
    assert_eq!(turn.as_deref(), Some(running.turn.as_str()));
    interrupted(&mut running, &[&written]);
    assert!(facts(&running).iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::Terminal {
            cause: Some(gent_types::TurnTerminalCause::Steered),
            ..
        }
    )));
    assert!(withdrawn(&running).is_empty());
    let first_released = facts(&running).into_iter().find_map(|fact| match fact {
        ConversationActivityFact::PromptReleased { message_id, .. } => Some(message_id),
        _ => None,
    });
    assert_eq!(first_released, Some(pressed.message.message_id.clone()));
    let state = running.runner.state.lock().unwrap();
    assert_eq!(state.starts + state.resumes, 2);
}
