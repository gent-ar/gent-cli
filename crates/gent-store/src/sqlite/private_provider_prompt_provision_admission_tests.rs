use gent_ports::{AgentChatPromptDispatchLedger, ConversationActivityLedger, PromptAdmission};
use gent_types::{ConversationActivityFact, HostEpoch, PromptHoldReason};

use super::test_support::{provision, seeded, settle};

fn count(ledger: &super::SqliteLedger, name: &str) -> usize {
    ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts
        .iter()
        .filter(|fact: &&ConversationActivityFact| {
            serde_json::to_value(fact).unwrap()["type"] == name
        })
        .count()
}

#[test]
fn an_install_hold_reports_held_then_installing_then_admitted_and_clears_once() {
    let (ledger, saved) = seeded();
    ledger
        .hold_agent_chat_prompt_for_admission(
            &saved.receipt.receipt_id,
            HostEpoch(1),
            PromptHoldReason::ProviderInstall,
        )
        .unwrap();
    assert_eq!(
        ledger
            .agent_chat_prompt_admission(&saved.receipt.receipt_id)
            .unwrap(),
        PromptAdmission::Held
    );

    let (binding, command, receipt) = provision(&ledger, &saved);
    assert_eq!(
        ledger
            .agent_chat_prompt_admission(&saved.receipt.receipt_id)
            .unwrap(),
        PromptAdmission::Installing
    );
    assert_eq!(count(&ledger, "promptReleased"), 0);

    settle(&ledger, &binding, &command, &receipt).unwrap();
    assert_eq!(
        ledger
            .agent_chat_prompt_admission(&saved.receipt.receipt_id)
            .unwrap(),
        PromptAdmission::Admitted
    );
    assert_eq!(count(&ledger, "promptHeld"), 1);
    assert_eq!(count(&ledger, "promptReleased"), 1);
    assert_eq!(count(&ledger, "promptCanceled"), 0);
}
