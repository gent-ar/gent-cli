use super::*;
use gent_types::{Command, ConversationActivityFact};

fn facts(scenario: &Scenario) -> Vec<ConversationActivityFact> {
    scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts
}

fn phase_of(scenario: &Scenario, turn: &str) -> DurableTurnPhase {
    scenario.ledger.find_turn(turn).unwrap().unwrap().phase
}

fn withdrawn(scenario: &Scenario) -> Vec<String> {
    facts(scenario)
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptWithdrawn { message_id, .. } => Some(message_id),
            _ => None,
        })
        .collect()
}

fn released(scenario: &Scenario, message_id: &str) -> bool {
    facts(scenario).iter().any(|fact| {
        matches!(fact, ConversationActivityFact::PromptReleased { message_id: released, .. }
            if released == message_id)
    })
}

fn stop(scenario: &mut Scenario) {
    scenario
        .ledger
        .stop_agent_chat_conversation(
            &Command {
                receipt_id: ReceiptId("stop".into()),
                idempotency_key: "agent-chat-interrupt:stop".into(),
                host_epoch: HostEpoch(1),
                kind: "agentChatInterrupt".into(),
                payload: serde_json::json!({"conversationId": "conversation", "runId": "run"}),
            },
            &AgentChatConversationId("conversation".into()),
        )
        .unwrap();
    scenario.host.interrupt("run").unwrap();
}

fn cancelled_by_claude(scenario: &mut Scenario, messages: &[&AgentChatPromptSaved]) {
    let mut effects = messages
        .iter()
        .map(|prompt| ClaudeRunnerEffect::MessageCancelled {
            message_id: prompt.message.message_id.clone(),
        })
        .collect::<Vec<_>>();
    effects.push(phase(TurnPhase::Interrupted));
    scenario.runner.0.lock().unwrap().effects.push_back(effects);
    scenario.host.tick().unwrap();
    scenario.host.tick().unwrap();
}

fn launches(scenario: &Scenario) -> (usize, usize) {
    let state = scenario.runner.0.lock().unwrap();
    (state.starts + state.resumes, state.protocol_interrupts)
}

#[test]
fn a_prompt_written_before_the_turn_ends_is_answered_as_claudes_next_turn() {
    let mut scenario = joined_scenario();
    let queued = scenario.queued.clone();
    scenario
        .runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![output("first"), phase(TurnPhase::Ready)]);
    scenario.host.tick().unwrap();
    assert_eq!(
        phase_of(&scenario, &queued.message.turn_id),
        DurableTurnPhase::Active
    );
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::MessageStarted {
            message_id: queued.message.message_id.clone(),
        },
        output("QUEUED-TWO"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    assert_eq!(
        phase_of(&scenario, &queued.message.turn_id),
        DurableTurnPhase::Completed
    );
    assert_eq!(turn_of(&scenario, "QUEUED-TWO"), queued.message.turn_id);
    assert!(released(&scenario, &queued.message.message_id));
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts, state.submits), (1, 1, 0));
}

#[test]
fn every_prompt_written_into_a_turn_that_ends_is_answered_without_a_resend() {
    let mut scenario = joined_scenario();
    let queued = scenario.queued.clone();
    let second = save(
        &scenario.ledger,
        "second",
        AgentChatPromptDisposition::Queue,
    );
    scenario.host.tick().unwrap();
    assert_eq!(scenario.runner.0.lock().unwrap().steers.len(), 2);
    scenario
        .runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![phase(TurnPhase::Ready)]);
    scenario.host.tick().unwrap();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::MessageStarted {
            message_id: queued.message.message_id.clone(),
        },
        ClaudeRunnerEffect::MessageStarted {
            message_id: second.message.message_id.clone(),
        },
        output("both"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    assert_eq!(turn_of(&scenario, "both"), queued.message.turn_id);
    assert!(facts(&scenario).iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptSteered { message_id, scope, .. }
            if message_id == &second.message.message_id && scope.turn_id == queued.message.turn_id)));
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts, state.submits), (2, 1, 0));
}

#[test]
fn stop_withdraws_a_written_prompt_claude_cancels_and_launches_nothing_after() {
    let mut scenario = joined_scenario();
    let held = save(&scenario.ledger, "held", AgentChatPromptDisposition::Queue);
    stop(&mut scenario);
    assert_eq!(
        withdrawn(&scenario),
        std::slice::from_ref(&held.message.message_id)
    );
    let queued = scenario.queued.clone();
    cancelled_by_claude(&mut scenario, &[&queued]);
    assert_eq!(
        phase_of(&scenario, &scenario.active.message.turn_id),
        DurableTurnPhase::Interrupted
    );
    assert_eq!(
        withdrawn(&scenario),
        [
            held.message.message_id.clone(),
            queued.message.message_id.clone()
        ]
    );
    assert!(!released(&scenario, &queued.message.message_id));
    assert!(!released(&scenario, &held.message.message_id));
    assert!(!facts(&scenario).iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::Terminal {
            cause: Some(gent_types::TurnTerminalCause::Steered),
            ..
        }
    )));
    for _ in 0..3 {
        scenario.host.tick().unwrap();
    }
    assert_eq!(launches(&scenario), (1, 1));
    assert_eq!(scenario.runner.0.lock().unwrap().steers.len(), 1);
}

#[test]
fn stop_before_the_session_starts_withdraws_the_prompt_it_never_wrote() {
    let mut scenario = queued_before_session();
    scenario.host.tick().unwrap();
    assert!(scenario.runner.0.lock().unwrap().steers.is_empty());
    stop(&mut scenario);
    cancelled_by_claude(&mut scenario, &[]);
    assert_eq!(
        withdrawn(&scenario),
        [scenario.queued.message.message_id.clone()]
    );
    assert_eq!(
        phase_of(&scenario, &scenario.queued.message.turn_id),
        DurableTurnPhase::Cancelled
    );
    assert_eq!(launches(&scenario), (1, 1));
    assert!(scenario.runner.0.lock().unwrap().steers.is_empty());
}

#[test]
fn a_prompt_claude_took_up_before_stop_stays_delivered_and_is_not_withdrawn() {
    let mut scenario = joined_scenario();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::MessageStarted {
            message_id: scenario.queued.message.message_id.clone(),
        },
        output("taken"),
    ]);
    scenario.host.tick().unwrap();
    stop(&mut scenario);
    cancelled_by_claude(&mut scenario, &[]);
    assert!(facts(&scenario).iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptSteered { message_id, .. }
            if message_id == &scenario.queued.message.message_id)));
    assert!(withdrawn(&scenario).is_empty());
    assert_eq!(launches(&scenario).0, 1);
}

#[test]
fn send_now_returns_written_prompts_to_the_queue_behind_the_pressed_one() {
    let mut scenario = joined_scenario();
    let pressed = save(
        &scenario.ledger,
        "pressed",
        AgentChatPromptDisposition::Queue,
    );
    let (_, _, turn) = scenario
        .ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("send-now".into()),
            HostEpoch(1),
            &AgentChatConversationId("conversation".into()),
            &pressed.message.message_id,
        )
        .unwrap();
    assert_eq!(
        turn.as_deref(),
        Some(scenario.active.message.turn_id.as_str())
    );
    scenario.host.interrupt("run").unwrap();
    let queued = scenario.queued.clone();
    cancelled_by_claude(&mut scenario, &[&queued]);
    assert!(facts(&scenario).iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::Terminal {
            cause: Some(gent_types::TurnTerminalCause::Steered),
            ..
        }
    )));
    assert!(withdrawn(&scenario).is_empty());
    assert!(released(&scenario, &pressed.message.message_id));
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!(state.submits, 1);
    assert_eq!(state.steers.last().unwrap().0, queued.message.message_id);
}
