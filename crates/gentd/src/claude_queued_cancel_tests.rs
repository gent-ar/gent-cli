use super::*;
use gent_ports::LedgerError;
use gent_types::{AgentChatRejection, ConversationActivityFact};

fn cancel(scenario: &Scenario, message_id: &str) -> Result<gent_types::Receipt, LedgerError> {
    scenario.ledger.cancel_queued_agent_chat_prompt(
        &ReceiptId(format!("cancel-{message_id}")),
        HostEpoch(1),
        &AgentChatConversationId("conversation".into()),
        message_id,
    )
}

fn outcome(scenario: &Scenario, message: &str) -> Vec<&'static str> {
    scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptCanceled { message_id, .. }
                if message_id == message =>
            {
                Some("canceled")
            }
            ConversationActivityFact::PromptWithdrawn { message_id, .. }
                if message_id == message =>
            {
                Some("withdrawn")
            }
            ConversationActivityFact::PromptSteered { message_id, .. } if message_id == message => {
                Some("steered")
            }
            _ => None,
        })
        .collect()
}

#[test]
fn removing_a_prompt_written_into_the_turn_withdraws_it_from_claude_and_cancels_it() {
    let mut scenario = joined_scenario();
    let queued = scenario.queued.clone();
    cancel(&scenario, &queued.message.message_id).unwrap();
    assert!(outcome(&scenario, &queued.message.message_id).is_empty());
    scenario.host.tick().unwrap();
    scenario.host.tick().unwrap();
    let withdrawals = scenario.runner.0.lock().unwrap().withdrawals.clone();
    assert!(!withdrawals.is_empty());
    assert!(
        withdrawals
            .iter()
            .all(|id| id == &queued.message.message_id)
    );
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::MessageCancelled {
            message_id: queued.message.message_id.clone(),
        },
    ]);
    scenario.host.tick().unwrap();
    assert_eq!(outcome(&scenario, &queued.message.message_id), ["canceled"]);
    let phase = |turn: &str| scenario.ledger.find_turn(turn).unwrap().unwrap().phase;
    assert_eq!(phase(&queued.message.turn_id), DurableTurnPhase::Cancelled);
    assert_eq!(
        phase(&scenario.active.message.turn_id),
        DurableTurnPhase::Active
    );
    for _ in 0..3 {
        scenario.host.tick().unwrap();
    }
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts, state.submits), (1, 1, 0));
}

#[test]
fn removing_a_prompt_claude_already_took_up_is_rejected_and_it_stays_delivered() {
    let mut scenario = joined_scenario();
    let queued = scenario.queued.clone();
    scenario
        .runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::MessageStarted {
            message_id: queued.message.message_id.clone(),
        }]);
    scenario.host.tick().unwrap();
    assert!(matches!(
        cancel(&scenario, &queued.message.message_id),
        Err(LedgerError::Rejected(
            AgentChatRejection::QueuedPromptNotCancelable
        ))
    ));
    scenario.host.tick().unwrap();
    assert!(scenario.runner.0.lock().unwrap().withdrawals.is_empty());
    assert_eq!(outcome(&scenario, &queued.message.message_id), ["steered"]);
}
