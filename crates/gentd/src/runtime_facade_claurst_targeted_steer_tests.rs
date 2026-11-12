use gent_ports::ConversationActivityLedger;
use gent_protocol::AgentChatSteerPrompt;
use gent_types::{ConversationActivityFact, DurableTurnPhase, TurnPhase, TurnTerminalCause};

use super::*;
use crate::agent_chat_intent_error::AgentChatIntentError;

fn targeted(
    h: &Harness,
    receipt: &str,
    message: &AgentChatPromptSaved,
    run: &str,
    turn: &str,
) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
    h.runtime
        .agent_chat_intent(AgentChatIntentFrame::SteerQueuedPromptForTurn {
            request_id: AgentChatRequestId(format!("request-{receipt}")),
            receipt_id: ReceiptId(receipt.into()),
            conversation_id: h.conversation_id.clone(),
            message_id: message.message.message_id.clone(),
            expected_run_id: AgentChatRunId(run.into()),
            expected_turn_id: turn.into(),
        })
}

fn atomic(
    h: &Harness,
    receipt: &str,
    turn: &str,
) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
    h.runtime
        .agent_chat_intent(AgentChatIntentFrame::SteerPromptForTurn(
            AgentChatSteerPrompt {
                request_id: AgentChatRequestId(format!("request-{receipt}")),
                receipt_id: ReceiptId(receipt.into()),
                conversation_id: h.conversation_id.clone(),
                expected_run_id: h.run_id.clone(),
                expected_turn_id: turn.into(),
                text: "steer now".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            },
        ))
}

fn settle(h: &Harness, active: &AgentChatPromptSaved, phase: DurableTurnPhase) {
    h.ledger
        .settle_agent_chat_prompt_terminal(
            &active.message.message_id,
            "daemon",
            HostEpoch(1),
            phase,
        )
        .unwrap();
}

fn next_turns(h: &Harness) -> Vec<String> {
    std::iter::from_fn(|| {
        h.ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Claurst)
            .unwrap()
            .map(|saved| saved.message.message_id)
    })
    .collect()
}

fn facts(h: &Harness) -> Vec<ConversationActivityFact> {
    h.ledger
        .read_conversation_activity_page(&h.conversation_id.0, &h.run_id.0, 0, 50)
        .unwrap()
        .facts
}

fn steer_rejected(h: &Harness) -> bool {
    facts(h)
        .iter()
        .any(|fact| matches!(fact, ConversationActivityFact::PromptSteerRejected { .. }))
}

fn goal_status(h: &Harness) -> (GoalStatus, GoalStatusReason) {
    let goal = h
        .ledger
        .current_goal(&h.conversation_id.0)
        .unwrap()
        .unwrap();
    (goal.status, goal.reason)
}

#[tokio::test]
async fn a_claurst_steer_interrupts_without_pausing_the_goal_while_a_user_stop_pauses_it() {
    let h = harness().await;
    set_goal(&h.ledger, &h.conversation_id);
    let active = running(&h.ledger, &h.conversation_id);
    let queued = prompt(
        &h.ledger,
        &h.conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    let steered = h
        .runtime
        .agent_chat_intent(AgentChatIntentFrame::SteerQueuedPrompt {
            request_id: AgentChatRequestId("steer".into()),
            receipt_id: ReceiptId("steer-receipt".into()),
            conversation_id: h.conversation_id.clone(),
            message_id: queued.message.message_id.clone(),
        })
        .unwrap();
    assert!(matches!(
        steered.as_slice(),
        [AgentChatIntentFrame::QueuedPromptSteered { .. }]
    ));
    assert_eq!(h.interrupted().await, vec![h.run_id.0.clone()]);
    assert_eq!(goal_status(&h).0, GoalStatus::Active);
    settle(&h, &active, DurableTurnPhase::Interrupted);
    assert_eq!(
        terminal_fact(&h.ledger, &h.conversation_id.0, &h.run_id.0),
        Some((TurnPhase::Interrupted, Some(TurnTerminalCause::Steered)))
    );
    h.runtime
        .agent_chat_intent(AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId("stop".into()),
            receipt_id: ReceiptId("stop-receipt".into()),
            conversation_id: h.conversation_id.clone(),
            run_id: h.run_id.clone(),
            expected_turn_id: None,
        })
        .unwrap();
    assert_eq!(
        goal_status(&h),
        (GoalStatus::Paused, GoalStatusReason::UserStopped)
    );
}

#[tokio::test]
async fn targeted_claurst_steer_interrupts_the_turn_as_steered_and_runs_next_ahead_of_the_queue() {
    let h = harness().await;
    set_goal(&h.ledger, &h.conversation_id);
    let active = running(&h.ledger, &h.conversation_id);
    let turn = active.message.turn_id.clone();
    let queue = |key| {
        prompt(
            &h.ledger,
            &h.conversation_id,
            key,
            AgentChatPromptDisposition::Queue,
        )
    };
    let (earlier, steered, later) = (queue("earlier"), queue("steered"), queue("later"));

    let reply = targeted(&h, "steer", &steered, &h.run_id.0, &turn).unwrap();
    assert!(matches!(
        reply.as_slice(),
        [AgentChatIntentFrame::QueuedPromptSteered { receipt, message_id, .. }]
            if receipt.receipt_id.0 == "steer" && message_id == &steered.message.message_id
    ));
    let retried = targeted(&h, "steer", &steered, &h.run_id.0, &turn).unwrap();
    assert!(matches!(
        retried.as_slice(),
        [AgentChatIntentFrame::QueuedPromptSteered { receipt, .. }] if receipt.receipt_id.0 == "steer"
    ));
    assert!(targeted(&h, "steer", &steered, &h.run_id.0, "another-turn").is_err());
    assert_eq!(h.interrupted().await[0], format!("{}/{turn}", h.run_id.0));
    assert_eq!(goal_status(&h).0, GoalStatus::Active);

    settle(&h, &active, DurableTurnPhase::Interrupted);
    assert_eq!(
        terminal_fact(&h.ledger, &h.conversation_id.0, &h.run_id.0),
        Some((TurnPhase::Interrupted, Some(TurnTerminalCause::Steered)))
    );
    assert!(!steer_rejected(&h));
    assert_eq!(
        next_turns(&h),
        [steered, earlier, later].map(|saved| saved.message.message_id)
    );
}

#[tokio::test]
async fn atomic_claurst_steer_runs_next_even_when_the_turn_ends_before_the_interrupt_lands() {
    let h = harness().await;
    let active = running(&h.ledger, &h.conversation_id);
    let turn = active.message.turn_id.clone();
    let earlier = prompt(
        &h.ledger,
        &h.conversation_id,
        "earlier",
        AgentChatPromptDisposition::Queue,
    );

    let reply = atomic(&h, "atomic", &turn).unwrap();
    let [
        AgentChatIntentFrame::Accepted {
            message_id,
            run_id,
            receipt,
            ..
        },
    ] = reply.as_slice()
    else {
        panic!("atomic steer must be accepted: {reply:?}")
    };
    assert_eq!(
        (run_id, receipt.receipt_id.0.as_str()),
        (&h.run_id, "atomic")
    );
    assert!(matches!(
        atomic(&h, "atomic", &turn).unwrap().as_slice(),
        [AgentChatIntentFrame::Accepted { message_id: replayed, .. }] if replayed == message_id
    ));
    assert_eq!(h.interrupted().await[0], format!("{}/{turn}", h.run_id.0));

    settle(&h, &active, DurableTurnPhase::Completed);
    assert!(!steer_rejected(&h));
    h.ledger
        .release_agent_chat_prompt_after_readiness(message_id, &h.run_id, HostEpoch(1))
        .unwrap();
    assert_eq!(
        next_turns(&h),
        [message_id.clone(), earlier.message.message_id.clone()]
    );

    let ended = atomic(&h, "after-end", &turn).unwrap_err();
    assert_eq!(ended.code, "queuedPromptNotSteerable");
    assert!(next_turns(&h).is_empty());
}

#[tokio::test]
async fn claurst_steer_for_a_wrong_turn_or_run_is_rejected_with_a_typed_code() {
    let h = harness().await;
    let active = running(&h.ledger, &h.conversation_id);
    let turn = active.message.turn_id.clone();
    let queued = prompt(
        &h.ledger,
        &h.conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );

    for (receipt, run, expected_turn) in [
        ("wrong-turn", h.run_id.0.as_str(), "not-the-turn"),
        ("wrong-run", "not-the-run", turn.as_str()),
    ] {
        let error = targeted(&h, receipt, &queued, run, expected_turn).unwrap_err();
        assert_eq!(error.code, "queuedPromptNotSteerable", "{receipt}");
    }
    let wrong_turn = atomic(&h, "atomic-wrong-turn", "not-the-turn").unwrap_err();
    assert_eq!(wrong_turn.code, "queuedPromptNotSteerable");
    assert!(h.interrupts.lock().unwrap().is_empty());
    settle(&h, &active, DurableTurnPhase::Completed);
    assert_eq!(
        next_turns(&h),
        std::slice::from_ref(&queued.message.message_id)
    );
}

fn switch_mid_turn(h: &Harness) -> AgentChatRunId {
    let switched = h
        .runtime
        .agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId("switch".into()),
            receipt_id: ReceiptId("switch-receipt".into()),
            conversation_id: h.conversation_id.clone(),
            parent_run_id: h.run_id.clone(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: "qwen3-1-7b-q4-k-m".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Ask,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        })
        .unwrap();
    let [AgentChatIntentFrame::Switched { run_id, .. }] = switched.as_slice() else {
        panic!("a mid-turn switch must be accepted")
    };
    assert_ne!(run_id, &h.run_id);
    run_id.clone()
}

#[tokio::test]
async fn stopping_the_current_run_after_a_mid_turn_switch_stops_the_running_turn() {
    let h = harness().await;
    running(&h.ledger, &h.conversation_id);
    let current = switch_mid_turn(&h);
    h.runtime
        .agent_chat_intent(AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId("stop".into()),
            receipt_id: ReceiptId("stop-receipt".into()),
            conversation_id: h.conversation_id.clone(),
            run_id: current,
            expected_turn_id: None,
        })
        .unwrap();
    assert_eq!(h.interrupted().await, vec![h.run_id.0.clone()]);
}

#[tokio::test]
async fn send_now_after_a_mid_turn_switch_steers_the_running_turn() {
    let h = harness().await;
    running(&h.ledger, &h.conversation_id);
    switch_mid_turn(&h);
    let queued = prompt(
        &h.ledger,
        &h.conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    let steered = h
        .runtime
        .agent_chat_intent(AgentChatIntentFrame::SteerQueuedPrompt {
            request_id: AgentChatRequestId("steer".into()),
            receipt_id: ReceiptId("steer-receipt".into()),
            conversation_id: h.conversation_id.clone(),
            message_id: queued.message.message_id.clone(),
        })
        .unwrap();
    assert!(matches!(
        steered.as_slice(),
        [AgentChatIntentFrame::QueuedPromptSteered { .. }]
    ));
    assert_eq!(h.interrupted().await, vec![h.run_id.0.clone()]);
}
