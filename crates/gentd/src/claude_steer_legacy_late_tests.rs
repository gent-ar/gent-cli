use super::*;

#[test]
fn a_take_up_after_the_result_in_the_same_batch_becomes_the_next_turn_without_a_resend() {
    let mut scenario = steered_scenario();
    let queued_id = scenario.queued.message.message_id.clone();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        output("first"),
        phase(TurnPhase::Ready),
        ClaudeRunnerEffect::SteerConsumed {
            message_id: queued_id,
        },
        output("second"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();

    let queued_turn = scenario.queued.message.turn_id.clone();
    assert_eq!(
        scenario
            .ledger
            .find_turn(&scenario.active.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        scenario
            .ledger
            .find_turn(&queued_turn)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        kinds(&scenario),
        [
            ("promptQueued".into(), queued_turn.clone()),
            ("promptSteerRequested".into(), queued_turn.clone()),
            ("promptReleased".into(), queued_turn.clone())
        ]
    );
    assert_eq!(turn_of(&scenario, "second"), queued_turn);
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts), (1, 1));
}
