#[path = "steer_target/support.rs"]
mod support;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatQueuedPromptLedger, ConversationActivityLedger, Ledger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatPromptDisposition as Disposition, AgentChatPromptSaved,
    AgentChatRunId, ConversationActivityFact, DurableTurnPhase, HostEpoch, ReceiptId,
};
use support::*;

fn request(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    queued: &AgentChatPromptSaved,
    active: &AgentChatPromptSaved,
    receipt: &str,
) -> Result<gent_types::Receipt, gent_ports::LedgerError> {
    ledger
        .steer_queued_agent_chat_prompt_for_turn(
            &ReceiptId(receipt.into()),
            HostEpoch(1),
            conversation,
            &queued.message.message_id,
            &active.run_id,
            &active.message.turn_id,
        )
        .map(|(r, _)| r)
}

fn claim(ledger: &SqliteLedger, active: &AgentChatPromptSaved) -> Option<AgentChatPromptSaved> {
    let claimed = ledger
        .claim_steered_agent_chat_prompt_for_turn(
            DAEMON,
            HostEpoch(1),
            &active.run_id,
            &active.message.turn_id,
        )
        .unwrap();
    if let Some(prompt) = &claimed {
        ledger
            .begin_agent_chat_prompt_launch(&prompt.message.message_id, DAEMON, HostEpoch(1))
            .unwrap();
    }
    claimed
}

fn finish(ledger: &SqliteLedger, active: &AgentChatPromptSaved) {
    ledger
        .settle_agent_chat_prompt_terminal(
            &active.message.message_id,
            DAEMON,
            HostEpoch(1),
            DurableTurnPhase::Completed,
        )
        .unwrap();
}

#[test]
fn rejects_stale_owner_turn_run_and_conversation_without_consuming_queue() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    for (conversation, run, turn) in [
        (
            conversation.clone(),
            current.run_id.clone(),
            "stale-turn".into(),
        ),
        (
            conversation.clone(),
            AgentChatRunId("other-run".into()),
            current.message.turn_id.clone(),
        ),
        (
            AgentChatConversationId("other-conversation".into()),
            current.run_id.clone(),
            current.message.turn_id.clone(),
        ),
    ] {
        assert!(
            ledger
                .steer_queued_agent_chat_prompt_for_turn(
                    &ReceiptId(format!("rejected-{run:?}-{turn}-{conversation:?}")),
                    HostEpoch(1),
                    &conversation,
                    &queued.message.message_id,
                    &run,
                    &turn
                )
                .is_err()
        );
    }
    assert!(request(&ledger, &conversation, &queued, &current, "valid").is_ok());
    let (_, run, turn) = ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("legacy-targetless".into()),
            HostEpoch(1),
            &conversation,
            &queued.message.message_id,
        )
        .unwrap();
    assert_eq!(run, current.run_id);
    assert_eq!(turn.as_deref(), Some(current.message.turn_id.as_str()));
    assert!(claim(&ledger, &current).is_none());
    ledger
        .settle_agent_chat_prompt_terminal(
            &current.message.message_id,
            DAEMON,
            HostEpoch(1),
            DurableTurnPhase::Interrupted,
        )
        .unwrap();
    let next = ledger
        .claim_agent_chat_prompt_dispatch(
            DAEMON,
            HostEpoch(1),
            gent_types::AgentChatProvider::Codex,
        )
        .unwrap();
    assert_eq!(next.unwrap().message, queued.message);
}

#[test]
fn terminal_after_acceptance_before_claim_runs_the_prompt_next_without_a_rejection() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    let receipt = request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
    finish(&ledger, &current);
    assert!(claim(&ledger, &current).is_none());
    assert_eq!(
        request(&ledger, &conversation, &queued, &current, "accepted").unwrap(),
        receipt
    );
    assert!(
        request(
            &ledger,
            &conversation,
            &queued,
            &current,
            "new-stale-request"
        )
        .is_err()
    );
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(
        !facts
            .iter()
            .any(|f| matches!(f, ConversationActivityFact::PromptSteerRejected { .. }))
    );
    assert!(
        !facts
            .iter()
            .any(|f| matches!(f, ConversationActivityFact::PromptSteered { .. }))
    );
    let pending = ledger
        .claim_agent_chat_prompt_dispatch(
            DAEMON,
            HostEpoch(1),
            gent_types::AgentChatProvider::Codex,
        )
        .unwrap();
    assert_eq!(pending.unwrap().message, queued.message);
}

#[test]
fn send_now_keeps_prompts_out_of_the_turn_it_interrupts_only() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
    assert!(claim(&ledger, &current).is_none());
    assert!(
        request(
            &ledger,
            &conversation,
            &queued,
            &current,
            "explicit-second-click"
        )
        .is_ok()
    );
    assert!(claim(&ledger, &current).is_none());
    assert_eq!(
        ledger
            .claim_steered_agent_chat_prompt_for_turn(
                DAEMON,
                HostEpoch(1),
                &current.run_id,
                "newer-turn"
            )
            .unwrap()
            .map(|prompt| prompt.message),
        Some(queued.message)
    );
}

#[test]
fn a_joined_prompt_is_delivered_only_into_a_turn_of_its_run() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    claim(&ledger, &current).unwrap();
    assert!(
        ledger
            .deliver_steered_agent_chat_prompt(
                &queued.message.message_id,
                DAEMON,
                HostEpoch(1),
                "newer-turn"
            )
            .is_err()
    );
    ledger
        .deliver_steered_agent_chat_prompt(
            &queued.message.message_id,
            DAEMON,
            HostEpoch(1),
            &current.message.turn_id,
        )
        .unwrap();
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|f|matches!(f,ConversationActivityFact::PromptSteered{scope,receipt_id,..} if scope.turn_id==current.message.turn_id && receipt_id=="receipt-queued")));
}

#[test]
fn a_joined_prompt_whose_turn_ended_starts_as_its_own_turn() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    claim(&ledger, &current).unwrap();
    finish(&ledger, &current);
    ledger
        .start_steered_agent_chat_prompt_turn(&queued.message.message_id, DAEMON, HostEpoch(1))
        .unwrap();
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(
        !facts
            .iter()
            .any(|f| matches!(f, ConversationActivityFact::PromptSteerRejected { .. }))
    );
    assert!(facts.iter().any(
        |f| matches!(f, ConversationActivityFact::PromptReleased { message_id, scope }
        if message_id == &queued.message.message_id && scope.turn_id == queued.message.turn_id)
    ));
}

#[test]
fn immutable_receipt_and_target_survive_reopen_and_duplicate_ack() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.db");
    let (ledger, conversation) = populate(SqliteLedger::open(&path).unwrap());
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    let receipt = request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
    drop(ledger);
    let ledger = SqliteLedger::open(&path).unwrap();
    assert_eq!(
        request(&ledger, &conversation, &queued, &current, "accepted").unwrap(),
        receipt
    );
    let mut wrong = current.clone();
    wrong.message.turn_id = "wrong-turn".into();
    assert!(request(&ledger, &conversation, &queued, &wrong, "accepted").is_err());
    assert!(claim(&ledger, &current).is_none());
    drop(ledger);
    let ledger = SqliteLedger::open(&path).unwrap();
    assert!(claim(&ledger, &current).is_none());
    ledger
        .settle_agent_chat_prompt_terminal(
            &current.message.message_id,
            DAEMON,
            HostEpoch(1),
            DurableTurnPhase::Interrupted,
        )
        .unwrap();
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::Terminal {
            cause: Some(gent_types::TurnTerminalCause::Steered),
            ..
        }
    )));
}

#[test]
fn a_joined_prompt_can_be_sent_now_and_waits_for_the_next_turn_once_returned() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    claim(&ledger, &current).unwrap();
    request(&ledger, &conversation, &queued, &current, "joined-send-now").unwrap();
    ledger
        .release_agent_chat_prompt_unstarted_launch(
            &queued.message.message_id,
            DAEMON,
            HostEpoch(1),
        )
        .unwrap();
    assert!(claim(&ledger, &current).is_none());
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(
        !facts
            .iter()
            .any(|f| matches!(f, ConversationActivityFact::PromptSteerRejected { .. }))
    );
}

#[test]
fn a_joined_prompt_returned_unconsumed_runs_as_the_next_queued_turn() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    claim(&ledger, &current).unwrap();
    ledger
        .release_agent_chat_prompt_unstarted_launch(
            &queued.message.message_id,
            DAEMON,
            HostEpoch(1),
        )
        .unwrap();
    finish(&ledger, &current);
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert_eq!(
        facts
            .iter()
            .filter(|f| matches!(f, ConversationActivityFact::PromptSteerRejected { .. }))
            .count(),
        0
    );
    let next = ledger
        .claim_agent_chat_prompt_dispatch(
            DAEMON,
            HostEpoch(1),
            gent_types::AgentChatProvider::Codex,
        )
        .unwrap();
    assert_eq!(next.unwrap().message, queued.message);
}

#[test]
fn a_joined_prompt_never_stands_in_for_the_live_dispatched_turn() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    prompt(&ledger, &conversation, "joined", Disposition::Queue);
    let pressed = prompt(&ledger, &conversation, "pressed", Disposition::Queue);
    claim(&ledger, &current).unwrap();
    let (_, _, turn) = ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("send-now".into()),
            HostEpoch(1),
            &conversation,
            &pressed.message.message_id,
        )
        .unwrap();
    assert_eq!(turn, Some(current.message.turn_id.clone()));
}

#[path = "steer_target/atomic.rs"]
mod atomic;
#[path = "steer_target/migration.rs"]
mod migration;
#[path = "steer_target/queue_delivery.rs"]
mod queue_delivery;
#[path = "steer_target/retarget.rs"]
mod retarget;
#[path = "steer_target/status_target.rs"]
mod status_target;

#[test]
fn a_steer_claimed_but_never_written_is_back_in_the_queue_after_a_restart() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    ledger
        .claim_steered_agent_chat_prompt_for_turn(
            DAEMON,
            HostEpoch(1),
            &original.run_id,
            &original.message.turn_id,
        )
        .unwrap()
        .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    let next = ledger
        .claim_agent_chat_prompt_dispatch(
            "new-owner",
            HostEpoch(2),
            gent_types::AgentChatProvider::Codex,
        )
        .unwrap()
        .unwrap();
    assert_eq!(next.message.message_id, queued.message.message_id);
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &original.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(
        !facts
            .iter()
            .any(|fact| matches!(fact, ConversationActivityFact::PromptSteerRejected { .. }))
    );
}

#[test]
fn a_released_in_turn_claim_returns_the_prompt_to_the_head_of_the_queue() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    let claim_raw = || {
        ledger
            .claim_steered_agent_chat_prompt_for_turn(
                DAEMON,
                HostEpoch(1),
                &original.run_id,
                &original.message.turn_id,
            )
            .unwrap()
            .map(|prompt| prompt.message.message_id)
    };
    let claimed = claim_raw().unwrap();
    ledger
        .release_agent_chat_prompt_claim(&claimed, DAEMON, HostEpoch(1))
        .unwrap();
    assert_eq!(claim_raw(), Some(queued.message.message_id));
}
