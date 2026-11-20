use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::ConversationActivityLedger;

use super::{Scenario, queued_before_session};

fn activity(scenario: &Scenario) -> Vec<serde_json::Value> {
    scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts
        .iter()
        .map(|fact| serde_json::to_value(fact).unwrap())
        .collect()
}

fn rejections(scenario: &Scenario) -> Vec<String> {
    activity(scenario)
        .iter()
        .filter(|fact| fact["type"] == "promptSteerRejected")
        .map(|fact| fact["reason"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_joined_prompt_whose_write_keeps_failing_is_rejected_once_and_never_reclaimed_into_the_turn() {
    let mut scenario = queued_before_session();
    scenario.runner.0.lock().unwrap().steer_writes_fail = true;
    scenario
        .runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "session".into(),
            },
        )]);
    scenario.host.tick().unwrap();
    assert_eq!(rejections(&scenario), ["deliveryRejected"]);
    let settled = activity(&scenario);
    for _ in 0..5 {
        scenario.host.tick().unwrap();
    }
    assert_eq!(activity(&scenario), settled);
    assert!(scenario.runner.0.lock().unwrap().steers.is_empty());
}
