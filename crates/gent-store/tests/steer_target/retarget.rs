use super::*;
use gent_ports::ConversationLedger;
use gent_ports::Ledger;

#[test]
fn successor_epoch_cannot_accept_or_claim_stale_owner_turn_before_recovery() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let first = prompt(&ledger, &conversation, "first", Disposition::Queue);
    let second = prompt(&ledger, &conversation, "second", Disposition::Queue);
    request(
        &ledger,
        &conversation,
        &first,
        &original,
        "old-host-accepted",
    )
    .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    assert!(
        ledger
            .steer_queued_agent_chat_prompt_for_turn(
                &ReceiptId("new-host-stale-turn".into()),
                HostEpoch(2),
                &conversation,
                &second.message.message_id,
                &original.run_id,
                &original.message.turn_id
            )
            .is_err()
    );
    assert!(
        ledger
            .claim_steered_agent_chat_prompt_for_turn(
                "new-owner",
                HostEpoch(2),
                &original.run_id,
                &original.message.turn_id
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn published_terminal_rejects_acceptance_and_claim_before_dispatch_phase_catches_up() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let first = prompt(&ledger, &conversation, "first", Disposition::Queue);
    let second = prompt(&ledger, &conversation, "second", Disposition::Queue);
    request(
        &ledger,
        &conversation,
        &first,
        &original,
        "accepted-before-terminal",
    )
    .unwrap();
    ledger
        .append_conversation_activity(&ConversationActivityFact::Terminal {
            scope: gent_types::ConversationActivityScope {
                conversation_id: conversation.0.clone(),
                run_id: original.run_id.0.clone(),
                turn_id: original.message.turn_id.clone(),
                host_epoch: HostEpoch(1),
                cursor: 999,
            },
            phase: gent_types::TurnPhase::Ready,
            cause: None,
        })
        .unwrap();
    assert_eq!(
        ledger
            .find_turn(&original.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Active
    );
    assert!(
        request(
            &ledger,
            &conversation,
            &second,
            &original,
            "stale-owner-after-terminal"
        )
        .is_err()
    );
    assert!(claim(&ledger, &original).is_none());
}

#[test]
fn host_epoch_recovery_rejects_an_ambiguous_target_without_replay() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    request(&ledger, &conversation, &queued, &original, "accepted").unwrap();
    claim(&ledger, &original).unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert!(
        ledger
            .claim_steered_agent_chat_prompt_for_turn(
                "new-owner",
                HostEpoch(2),
                &original.run_id,
                &original.message.turn_id
            )
            .unwrap()
            .is_none()
    );
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, &original.run_id.0, 0, 50)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|f| matches!(f,ConversationActivityFact::PromptSteerRejected{receipt_id,reason,..} if receipt_id=="accepted" && reason=="deliveryUnprovable")));
}

#[test]
fn a_steer_whose_turn_ended_waits_behind_an_older_unsteered_prompt() {
    let (ledger, conversation) = ledger();
    let original = active(&ledger, &conversation);
    let next = prompt(&ledger, &conversation, "next", Disposition::Queue);
    let queued = prompt(&ledger, &conversation, "queued", Disposition::Queue);
    request(&ledger, &conversation, &queued, &original, "original-click").unwrap();
    finish(&ledger, &original);
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch(
            DAEMON,
            HostEpoch(1),
            gent_types::AgentChatProvider::Codex,
        )
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message, next.message);
}
