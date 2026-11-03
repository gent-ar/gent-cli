use super::*;
use gent_ports::{AgentChatPromptLedger, AgentChatReadLedger, ConversationLedger};
use gent_types::{AgentChatPromptCreate, AgentChatRequestId};

fn input(conversation: &AgentChatConversationId) -> AgentChatPromptCreate {
    AgentChatPromptCreate {
        request_id: AgentChatRequestId("primary-steer".into()),
        receipt_id: ReceiptId("receipt-primary-steer".into()),
        host_epoch: HostEpoch(1),
        conversation_id: conversation.clone(),
        disposition: Disposition::Queue,
        text: "primary explicit steer".into(),
        attachment_ids: vec![],
        tool_source_ids: vec![],
    }
}

#[test]
fn a_primary_steer_cannot_auto_dispatch_if_its_target_ends_before_target_acceptance() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let create = input(&conversation);
    let queued = ledger
        .save_agent_chat_prompt_for_steer(&create, &original.run_id, &original.message.turn_id)
        .unwrap();
    finish(&ledger, &original);
    ledger
        .release_agent_chat_prompt_after_readiness(
            &queued.message.message_id,
            &queued.run_id,
            HostEpoch(1),
        )
        .unwrap();
    assert!(
        request(
            &ledger,
            &conversation,
            &queued,
            &original,
            "arrived-after-terminal"
        )
        .is_err()
    );
    let dispatched = ledger
        .claim_agent_chat_prompt_dispatch(
            DAEMON,
            HostEpoch(1),
            gent_types::AgentChatProvider::Codex,
        )
        .unwrap();
    assert!(
        dispatched.is_none(),
        "primary steer became ordinary dispatch: {dispatched:?}"
    );
    assert_eq!(
        ledger
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Active
    );
    let replay = ledger
        .save_agent_chat_prompt_for_steer(&create, &original.run_id, &original.message.turn_id)
        .unwrap();
    assert_eq!(replay.message, queued.message);
    assert_eq!(replay.receipt, queued.receipt);
    assert!(claim(&ledger, &original).is_none());
    assert!(
        ledger
            .save_agent_chat_prompt_for_steer(&create, &original.run_id, "newer-turn")
            .is_err()
    );
}

#[test]
fn atomic_primary_is_excluded_immediately_and_claims_only_its_original_target() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let create = input(&conversation);
    let queued = ledger
        .save_agent_chat_prompt_for_steer(&create, &original.run_id, &original.message.turn_id)
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(
            &queued.message.message_id,
            &queued.run_id,
            HostEpoch(1),
        )
        .unwrap();
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                DAEMON,
                HostEpoch(1),
                gent_types::AgentChatProvider::Codex
            )
            .unwrap()
            .is_none()
    );
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &original.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptSteerRequested {scope,message_id,receipt_id}
        if scope.turn_id==original.message.turn_id && message_id==&queued.message.message_id && receipt_id==&create.receipt_id.0)));
    let claimed = claim(&ledger, &original).unwrap();
    assert_eq!(claimed.0.message, queued.message);
    assert_eq!(claimed.1, create.receipt_id);
}

#[test]
fn target_expired_before_atomic_enqueue_rejects_without_a_prompt_or_dispatch() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    finish(&ledger, &original);
    let create = input(&conversation);
    assert!(
        ledger
            .save_agent_chat_prompt_for_steer(&create, &original.run_id, &original.message.turn_id)
            .is_err()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                DAEMON,
                HostEpoch(1),
                gent_types::AgentChatProvider::Codex
            )
            .unwrap()
            .is_none()
    );
    assert!(
        ledger
            .read_agent_chat_transcript(&conversation.0, None, 100)
            .unwrap()
            .events
            .iter()
            .all(|event| event.text != create.text)
    );
}

#[test]
fn atomic_validation_failure_rolls_back_message_receipt_and_target_together() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let mut create = input(&conversation);
    create.attachment_ids = vec!["missing-attachment".into()];
    assert!(
        ledger
            .save_agent_chat_prompt_for_steer(&create, &original.run_id, &original.message.turn_id)
            .is_err()
    );
    assert!(
        ledger
            .read_agent_chat_transcript(&conversation.0, None, 100)
            .unwrap()
            .events
            .iter()
            .all(|event| event.text != create.text)
    );
    create.attachment_ids.clear();
    let saved = ledger
        .save_agent_chat_prompt_for_steer(&create, &original.run_id, &original.message.turn_id)
        .unwrap();
    assert_eq!(saved.receipt.receipt_id, create.receipt_id);
}
