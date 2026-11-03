use super::*;
use gent_types::ConversationActivityFact;

#[test]
fn terminal_retires_all_targeted_writes_without_resending_any_of_them() {
    let mut scenario = scenario(true);
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
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        phase(TurnPhase::Ready),
        ClaudeRunnerEffect::SteerConsumed {
            message_id: scenario.queued.message.message_id.clone(),
        },
    ]);
    scenario.host.tick().unwrap();
    scenario.host.tick().unwrap();
    let facts = scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts;
    for (queued, receipt) in [(&scenario.queued, "steer"), (&second, "second-steer")] {
        assert_eq!(
            scenario
                .ledger
                .find_turn(&queued.message.turn_id)
                .unwrap()
                .unwrap()
                .phase,
            DurableTurnPhase::Failed
        );
        assert!(facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptSteerRejected {message_id,receipt_id,reason,..}
            if message_id==&queued.message.message_id && receipt_id==receipt && reason=="deliveryUnprovable")));
        assert!(!facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptReleased {message_id,..}|ConversationActivityFact::PromptSteered {message_id,..}
            if message_id==&queued.message.message_id)));
    }
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.starts, state.submits), (1, 0));
}

#[test]
fn targeted_take_up_after_terminal_is_rejected_instead_of_becoming_a_new_turn() {
    let mut scenario = scenario(true);
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        output("first"),
        phase(TurnPhase::Ready),
        ClaudeRunnerEffect::SteerConsumed {
            message_id: scenario.queued.message.message_id.clone(),
        },
    ]);
    scenario.host.tick().unwrap();
    let facts = kinds(&scenario);
    assert!(
        facts
            .iter()
            .any(|(kind, turn)| kind == "promptSteerRejected"
                && turn == &scenario.active.message.turn_id)
    );
    let queued_facts = scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts;
    assert!(
        !queued_facts.iter().any(|fact| matches!(fact,
            ConversationActivityFact::PromptSteered { message_id, .. } | ConversationActivityFact::PromptReleased { message_id, .. }
                if message_id == &scenario.queued.message.message_id)),
        "facts={facts:?}; starts={}; queued_phase={:?}",
        scenario.runner.0.lock().unwrap().starts,
        scenario.ledger.find_turn(&scenario.queued.message.turn_id).unwrap().unwrap().phase,
    );
    assert_eq!(
        scenario
            .ledger
            .find_turn(&scenario.queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Failed
    );
    assert!(queued_facts.iter().any(|fact| matches!(fact,
        ConversationActivityFact::PromptSteerRejected {message_id,receipt_id,reason,scope}
            if message_id == &scenario.queued.message.message_id && receipt_id == "steer"
                && reason == "deliveryUnprovable" && scope.turn_id == scenario.active.message.turn_id)));
    assert_eq!(scenario.runner.0.lock().unwrap().starts, 1);
    assert_eq!(scenario.runner.0.lock().unwrap().submits, 0);
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
