use super::*;
use gent_ports::NormalizedSessionBatchLedger;
use gent_types::{AgentChatProvider, AgentChatQueueDelivery};

fn delivery(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    queued: &AgentChatPromptSaved,
) -> Option<AgentChatQueueDelivery> {
    ledger
        .read_conversation_activity_page(&conversation.0, &queued.run_id.0, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .find_map(|fact| match fact {
            ConversationActivityFact::PromptQueued {
                message_id,
                queue_delivery,
                ..
            } if message_id == queued.message.message_id => queue_delivery,
            _ => None,
        })
}

fn queued_during_a_live_turn(provider: AgentChatProvider) -> Option<AgentChatQueueDelivery> {
    let (ledger, conversation) = populate_for(SqliteLedger::in_memory().unwrap(), provider);
    active_for(&ledger, &conversation, provider);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    delivery(&ledger, &conversation, &queued)
}

#[test]
fn a_prompt_queued_during_a_live_turn_says_whether_it_joins_that_turn() {
    assert_eq!(
        queued_during_a_live_turn(AgentChatProvider::Claude),
        Some(AgentChatQueueDelivery::JoinTurn)
    );
    assert_eq!(
        queued_during_a_live_turn(AgentChatProvider::Codex),
        Some(AgentChatQueueDelivery::JoinTurn)
    );
    assert_eq!(
        queued_during_a_live_turn(AgentChatProvider::Claurst),
        Some(AgentChatQueueDelivery::NextTurn)
    );
}

#[test]
fn a_prompt_queued_with_no_live_turn_waits_for_the_next_turn() {
    let (ledger, conversation) = populate_for(
        SqliteLedger::in_memory().unwrap(),
        AgentChatProvider::Claude,
    );
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    assert_eq!(
        delivery(&ledger, &conversation, &queued),
        Some(AgentChatQueueDelivery::NextTurn)
    );
}

#[test]
fn a_prompt_queued_during_a_provider_started_claude_turn_joins_it() {
    let (ledger, conversation) = populate_for(
        SqliteLedger::in_memory().unwrap(),
        AgentChatProvider::Claude,
    );
    ledger
        .start_provider_turn(&conversation.0, "run-1", HostEpoch(1))
        .unwrap();
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    assert_eq!(
        delivery(&ledger, &conversation, &queued),
        Some(AgentChatQueueDelivery::JoinTurn)
    );
}

#[test]
fn a_prompt_queued_behind_a_send_now_interrupt_waits_for_the_next_turn() {
    let (ledger, conversation) = populate_for(
        SqliteLedger::in_memory().unwrap(),
        AgentChatProvider::Claude,
    );
    let current = active_for(&ledger, &conversation, AgentChatProvider::Claude);
    let pressed = prompt(&ledger, &conversation, "pressed", Disposition::Queue);
    request(&ledger, &conversation, &pressed, &current, "send-now").unwrap();
    let behind = prompt(&ledger, &conversation, "behind", Disposition::Queue);
    assert_eq!(
        delivery(&ledger, &conversation, &behind),
        Some(AgentChatQueueDelivery::NextTurn)
    );
}

#[test]
fn the_queued_fact_carries_the_delivery_on_the_wire() {
    let (ledger, conversation) = populate_for(
        SqliteLedger::in_memory().unwrap(),
        AgentChatProvider::Claude,
    );
    active_for(&ledger, &conversation, AgentChatProvider::Claude);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    let fact = ledger
        .read_conversation_activity_page(&conversation.0, &queued.run_id.0, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .find(|fact| matches!(fact, ConversationActivityFact::PromptQueued { .. }))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&fact).unwrap()["queueDelivery"],
        "joinTurn"
    );
}
