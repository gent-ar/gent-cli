use super::*;
use gent_types::ConversationActivityFact;

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

fn assert_ran_as_its_own_turn(scenario: &Scenario, queued: &AgentChatPromptSaved) {
    let facts = facts(scenario);
    assert!(facts.iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptSteerRejected { message_id, reason, scope, .. }
            if message_id == &queued.message.message_id && reason == "targetExpired"
                && scope.turn_id == scenario.active.message.turn_id)));
    assert!(!facts.iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptSteerRejected { reason, .. } if reason == "deliveryUnprovable")));
    assert_eq!(
        phase_of(scenario, &queued.message.turn_id),
        DurableTurnPhase::Completed
    );
}

#[test]
fn a_targeted_steer_written_before_the_turn_ends_is_answered_as_claudes_next_turn() {
    let mut scenario = scenario(true);
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
        phase_of(&scenario, &scenario.active.message.turn_id),
        DurableTurnPhase::Completed
    );
    assert_eq!(
        phase_of(&scenario, &queued.message.turn_id),
        DurableTurnPhase::Active
    );
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::SteerConsumed {
            message_id: queued.message.message_id.clone(),
        },
        output("QUEUED-TWO"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    assert_ran_as_its_own_turn(&scenario, &queued);
    assert_eq!(turn_of(&scenario, "QUEUED-TWO"), queued.message.turn_id);
    assert_eq!(turn_of(&scenario, "first"), scenario.active.message.turn_id);
    assert!(facts(&scenario).iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptReleased { message_id, .. } if message_id == &queued.message.message_id)));
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts, state.submits), (1, 1, 0));
    drop(state);
    assert!(!scenario.host.needs_drive());
}

#[test]
fn a_targeted_take_up_reported_after_the_result_becomes_the_next_turn_without_a_resend() {
    let mut scenario = scenario(true);
    let queued = scenario.queued.clone();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        output("first"),
        phase(TurnPhase::Ready),
        ClaudeRunnerEffect::SteerConsumed {
            message_id: queued.message.message_id.clone(),
        },
        output("second"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    assert_ran_as_its_own_turn(&scenario, &queued);
    assert_eq!(turn_of(&scenario, "second"), queued.message.turn_id);
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts, state.submits), (1, 1, 0));
}

#[test]
fn every_targeted_write_outliving_its_turn_is_answered_without_a_resend() {
    let mut scenario = scenario(true);
    let queued = scenario.queued.clone();
    let second = save(
        &scenario.ledger,
        "second",
        AgentChatPromptDisposition::Queue,
    );
    scenario
        .ledger
        .steer_queued_agent_chat_prompt_for_turn(
            &ReceiptId("second-steer".into()),
            HostEpoch(1),
            &AgentChatConversationId("conversation".into()),
            &second.message.message_id,
            &scenario.active.run_id,
            &scenario.active.message.turn_id,
        )
        .unwrap();
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
        ClaudeRunnerEffect::SteerConsumed {
            message_id: queued.message.message_id.clone(),
        },
        ClaudeRunnerEffect::SteerConsumed {
            message_id: second.message.message_id.clone(),
        },
        output("both"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    assert_ran_as_its_own_turn(&scenario, &queued);
    assert_ran_as_its_own_turn(&scenario, &second);
    assert_eq!(turn_of(&scenario, "both"), queued.message.turn_id);
    assert!(facts(&scenario).iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptSteered { message_id, scope, .. }
            if message_id == &second.message.message_id && scope.turn_id == queued.message.turn_id)));
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts, state.submits), (2, 1, 0));
}

#[test]
fn targeted_take_up_before_terminal_records_only_the_captured_turn() {
    let mut scenario = scenario(true);
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::SteerConsumed {
            message_id: scenario.queued.message.message_id.clone(),
        },
        output("first"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();
    let facts = kinds(&scenario);
    assert!(facts.iter().any(|(kind,turn)| kind=="promptSteered" && turn==&scenario.active.message.turn_id));
    assert!(
        !facts
            .iter()
            .any(|(kind, _)| kind == "promptSteerRejected" || kind == "promptReleased")
    );
}
