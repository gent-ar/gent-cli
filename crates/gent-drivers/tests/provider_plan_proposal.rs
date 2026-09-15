use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame},
};
use gent_types::NormalizedProviderEvent;
use serde_json::json;

fn proposed(provider: PublicProvider, frame: &serde_json::Value) -> Vec<String> {
    normalize_public_frame(provider, frame)
        .into_iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Event(NormalizedProviderEvent::PlanProposed { text }) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn claude_exit_plan_mode_input_is_the_proposed_plan() {
    let frame = json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"ExitPlanMode","input":{"plan":"# Plan\n1. Add README.md"}}]}});
    assert_eq!(
        proposed(PublicProvider::Claude, &frame),
        ["# Plan\n1. Add README.md"]
    );
    let other = json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Write","input":{"plan":"not a plan"}}]}});
    assert!(proposed(PublicProvider::Claude, &other).is_empty());
}

#[test]
fn codex_completed_plan_item_is_the_proposed_plan() {
    let frame = json!({"method":"item/completed","params":{"threadId":"t","turnId":"u","item":{"type":"plan","id":"u-plan","text":"1. Create README.md\n2. Add usage\n"}}});
    assert_eq!(
        proposed(PublicProvider::Codex, &frame),
        ["1. Create README.md\n2. Add usage\n"]
    );
    let started =
        json!({"method":"item/started","params":{"item":{"type":"plan","id":"u-plan","text":""}}});
    assert!(proposed(PublicProvider::Codex, &started).is_empty());
}
