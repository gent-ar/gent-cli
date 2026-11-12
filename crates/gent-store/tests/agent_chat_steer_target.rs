#[path = "steer_target/support.rs"]
mod support;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatQueuedPromptLedger, ConversationActivityLedger,
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

fn claim(
    ledger: &SqliteLedger,
    active: &AgentChatPromptSaved,
) -> Option<(AgentChatPromptSaved, ReceiptId)> {
    ledger
        .claim_steered_agent_chat_prompt_for_turn(
            DAEMON,
            HostEpoch(1),
            &active.run_id,
            &active.message.turn_id,
        )
        .unwrap()
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
    assert!(
        ledger
            .steer_queued_agent_chat_prompt(
                &ReceiptId("legacy-targetless".into()),
                HostEpoch(1),
                &conversation,
                &queued.message.message_id
            )
            .is_err()
    );
    assert_eq!(claim(&ledger, &current).unwrap().0.message, queued.message);
}

#[test]
fn terminal_after_acceptance_before_claim_expires_once_and_retains_prompt() {
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
    assert_eq!(facts.iter().filter(|f| matches!(f,ConversationActivityFact::PromptSteerRejected { message_id,receipt_id,reason,.. }
        if message_id==&queued.message.message_id && receipt_id=="accepted" && reason=="targetExpired")).count(),1);
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
    assert!(pending.is_none());
}

#[test]
fn new_lifecycle_binding_cannot_claim_an_older_accepted_target() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
    assert!(
        ledger
            .claim_steered_agent_chat_prompt_for_turn(
                DAEMON,
                HostEpoch(1),
                &current.run_id,
                "newer-turn"
            )
            .unwrap()
            .is_none()
    );
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
    assert!(claim(&ledger, &current).is_some());
}

#[test]
fn delivery_and_legacy_fallback_cannot_change_target_after_claim() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
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
    assert!(
        ledger
            .start_steered_agent_chat_prompt_turn(&queued.message.message_id, DAEMON, HostEpoch(1))
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
    assert!(facts.iter().any(|f|matches!(f,ConversationActivityFact::PromptSteered{scope,receipt_id,..} if scope.turn_id==current.message.turn_id && receipt_id=="accepted")));
}

#[test]
fn a_claimed_target_whose_turn_ended_starts_as_its_own_turn() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
    claim(&ledger, &current).unwrap();
    finish(&ledger, &current);
    ledger
        .start_steered_agent_chat_prompt_turn(&queued.message.message_id, DAEMON, HostEpoch(1))
        .unwrap();
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &current.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|f| matches!(f, ConversationActivityFact::PromptSteerRejected { message_id, receipt_id, reason, scope }
        if message_id == &queued.message.message_id && receipt_id == "accepted" && reason == "targetExpired" && scope.turn_id == current.message.turn_id)));
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
    assert_eq!(claim(&ledger, &current).unwrap().1, receipt.receipt_id);
    drop(ledger);
    let ledger = SqliteLedger::open(&path).unwrap();
    assert!(
        ledger
            .deliver_steered_agent_chat_prompt(
                &queued.message.message_id,
                DAEMON,
                HostEpoch(1),
                "wrong-turn"
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
}

#[test]
fn released_claim_rejects_delivery_and_allows_explicit_new_receipt_only() {
    let (ledger, conversation) = ledger();
    let current = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    let receipt = request(&ledger, &conversation, &queued, &current, "accepted").unwrap();
    claim(&ledger, &current).unwrap();
    ledger
        .release_agent_chat_prompt_unstarted_launch(
            &queued.message.message_id,
            DAEMON,
            HostEpoch(1),
        )
        .unwrap();
    assert_eq!(
        request(&ledger, &conversation, &queued, &current, "accepted").unwrap(),
        receipt
    );
    assert!(claim(&ledger, &current).is_none());
    request(
        &ledger,
        &conversation,
        &queued,
        &current,
        "explicit-new-click",
    )
    .unwrap();
    assert_eq!(
        claim(&ledger, &current).unwrap().1,
        ReceiptId("explicit-new-click".into())
    );
}

#[path = "steer_target/atomic.rs"]
mod atomic;
#[path = "steer_target/migration.rs"]
mod migration;
#[path = "steer_target/retarget.rs"]
mod retarget;
