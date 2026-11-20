#[path = "support/queue_send_now_stop.rs"]
mod support;
use support::*;

#[test]
fn a_queued_prompt_joins_the_running_turn_in_queue_order_without_a_steer() {
    let ledger = ledger();
    let turn = running(&ledger);
    let first = prompt(&ledger, "first", AgentChatPromptDisposition::Queue, &[]);
    let second = prompt(&ledger, "second", AgentChatPromptDisposition::Queue, &[]);
    assert_eq!(join(&ledger, &turn), Some(first.message.message_id.clone()));
    assert_eq!(
        join(&ledger, &turn),
        Some(second.message.message_id.clone())
    );
    assert_eq!(join(&ledger, &turn), None);
    ledger
        .begin_agent_chat_prompt_launch(&first.message.message_id, DAEMON, EPOCH)
        .unwrap();
    ledger
        .deliver_steered_agent_chat_prompt(
            &first.message.message_id,
            DAEMON,
            EPOCH,
            &turn.message.turn_id,
        )
        .unwrap();
    let steered = facts(&ledger, "promptSteered");
    assert_eq!(steered.len(), 1);
    assert_eq!(steered[0]["messageId"], first.message.message_id.as_str());
    assert_eq!(steered[0]["receiptId"], "receipt-first");
    assert_eq!(steered[0]["turnId"], turn.message.turn_id.as_str());
}

#[test]
fn send_now_interrupts_the_turn_and_runs_its_prompt_first_as_the_next_turn() {
    let ledger = ledger();
    let turn = running(&ledger);
    let older = prompt(&ledger, "older", AgentChatPromptDisposition::Queue, &[]);
    let pressed = prompt(&ledger, "pressed", AgentChatPromptDisposition::Queue, &[]);
    let (_, run, interrupted) = ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("send-now".into()),
            EPOCH,
            &conversation(),
            &pressed.message.message_id,
        )
        .unwrap();
    assert_eq!(run, turn.run_id);
    assert_eq!(interrupted.as_deref(), Some(turn.message.turn_id.as_str()));
    assert_eq!(join(&ledger, &turn), None);
    end_turn(&ledger, &turn, DurableTurnPhase::Interrupted);
    let terminal = facts(&ledger, "terminal");
    assert_eq!(terminal.last().unwrap()["cause"], "steered");
    let next = ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    assert_eq!(next.message.message_id, pressed.message.message_id);
    let after = ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    assert_eq!(after.message.message_id, older.message.message_id);
}

#[test]
fn send_now_with_no_running_turn_only_releases_the_prompt() {
    let ledger = ledger();
    let queued = prompt(&ledger, "queued", AgentChatPromptDisposition::Queue, &[]);
    let (_, _, interrupted) = ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("send-now".into()),
            EPOCH,
            &conversation(),
            &queued.message.message_id,
        )
        .unwrap();
    assert_eq!(interrupted, None);
    let next = ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    assert_eq!(next.message.message_id, queued.message.message_id);
}

#[test]
fn stop_withdraws_every_held_prompt_in_queue_order_with_its_text_and_attachments() {
    let ledger = ledger();
    stage(&ledger, "attachment-1");
    let turn = running(&ledger);
    let first = prompt(&ledger, "first", AgentChatPromptDisposition::Queue, &[]);
    let second = prompt(
        &ledger,
        "second",
        AgentChatPromptDisposition::Queue,
        &["attachment-1"],
    );
    let receipt = stop(&ledger, "stop-1");
    let expected = vec![
        (
            first.message.message_id.clone(),
            "prompt first".to_owned(),
            json!([]),
        ),
        (
            second.message.message_id.clone(),
            "prompt second".to_owned(),
            json!([{"attachmentId": "attachment-1", "displayName": "diagram.png", "mediaType": "image/png", "byteLen": 5}]),
        ),
    ];
    assert_eq!(withdrawn(&ledger), expected);
    assert_eq!(join(&ledger, &turn), None);
    assert_eq!(stop(&ledger, "stop-1"), receipt);
    assert_eq!(withdrawn(&ledger).len(), 2);
    end_turn(&ledger, &turn, DurableTurnPhase::Interrupted);
    assert!(
        facts(&ledger, "terminal")
            .last()
            .unwrap()
            .get("cause")
            .is_none()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ledger
            .find_turn(&first.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Cancelled
    );
}

#[test]
fn stop_withdraws_a_prompt_written_into_the_turn_once_the_provider_returns_it() {
    let ledger = ledger();
    let turn = running(&ledger);
    let written = prompt(&ledger, "written", AgentChatPromptDisposition::Queue, &[]);
    let held = prompt(&ledger, "held", AgentChatPromptDisposition::Queue, &[]);
    assert_eq!(
        join(&ledger, &turn),
        Some(written.message.message_id.clone())
    );
    ledger
        .begin_agent_chat_prompt_launch(&written.message.message_id, DAEMON, EPOCH)
        .unwrap();
    stop(&ledger, "stop-1");
    let ids = |ledger: &SqliteLedger| {
        withdrawn(ledger)
            .into_iter()
            .map(|(message, _, _)| message)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&ledger), std::slice::from_ref(&held.message.message_id));
    ledger
        .release_agent_chat_prompt_unstarted_launch(&written.message.message_id, DAEMON, EPOCH)
        .unwrap();
    assert_eq!(
        ids(&ledger),
        [
            held.message.message_id.clone(),
            written.message.message_id.clone()
        ]
    );
    end_turn(&ledger, &turn, DurableTurnPhase::Interrupted);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
            .unwrap()
            .is_none()
    );
    ledger.close_ingress(EPOCH).unwrap();
    ledger.fence_and_open(EPOCH).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-b", HostEpoch(2), AgentChatProvider::Claude)
            .unwrap()
            .is_none()
    );
}

fn requested_turns(ledger: &SqliteLedger) -> Vec<String> {
    facts(ledger, "interruptRequested")
        .into_iter()
        .map(|fact| fact["turnId"].as_str().unwrap().to_owned())
        .collect()
}

fn cursor_of(ledger: &SqliteLedger, kind: &str) -> u64 {
    facts(ledger, kind)[0]["cursor"].as_u64().unwrap()
}

#[test]
fn stop_records_that_it_was_accepted_for_the_running_turn_in_its_own_transaction() {
    let ledger = ledger();
    let turn = running(&ledger);
    prompt(&ledger, "queued", AgentChatPromptDisposition::Queue, &[]);
    let receipt = stop(&ledger, "stop-1");
    assert_eq!(
        requested_turns(&ledger),
        std::slice::from_ref(&turn.message.turn_id)
    );
    assert!(cursor_of(&ledger, "interruptRequested") > cursor_of(&ledger, "promptWithdrawn"));
    assert_eq!(stop(&ledger, "stop-1"), receipt);
    assert_eq!(requested_turns(&ledger).len(), 1);
}

#[test]
fn stop_of_a_turn_still_launching_is_recorded_and_settles_it_interrupted() {
    let ledger = ledger();
    let launching = prompt(&ledger, "send", AgentChatPromptDisposition::Send, &[]);
    ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    stop(&ledger, "stop-1");
    assert_eq!(
        requested_turns(&ledger),
        std::slice::from_ref(&launching.message.turn_id)
    );
    assert!(
        ledger
            .interrupt_unstarted_agent_chat_prompt("run-1", None, EPOCH)
            .unwrap()
    );
    assert_eq!(
        ledger
            .find_turn(&launching.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Interrupted
    );
}

#[test]
fn stop_names_only_the_turn_it_stops_and_not_a_prompt_written_into_it() {
    let ledger = ledger();
    let turn = running(&ledger);
    let written = prompt(&ledger, "written", AgentChatPromptDisposition::Queue, &[]);
    assert_eq!(
        join(&ledger, &turn),
        Some(written.message.message_id.clone())
    );
    stop(&ledger, "stop-1");
    assert_eq!(
        requested_turns(&ledger),
        std::slice::from_ref(&turn.message.turn_id)
    );
}

#[test]
fn stop_with_nothing_running_records_no_interrupt_request() {
    let ledger = ledger();
    let turn = running(&ledger);
    end_turn(&ledger, &turn, DurableTurnPhase::Completed);
    stop(&ledger, "stop-1");
    assert!(requested_turns(&ledger).is_empty());
}

fn cancel(
    ledger: &SqliteLedger,
    message_id: &str,
) -> Result<gent_types::Receipt, gent_ports::LedgerError> {
    ledger.cancel_queued_agent_chat_prompt(
        &ReceiptId(format!("cancel-{message_id}")),
        EPOCH,
        &conversation(),
        message_id,
    )
}

fn queued_cancelable(ledger: &SqliteLedger) -> Vec<(Value, Value)> {
    facts(ledger, "promptQueued")
        .into_iter()
        .map(|fact| (fact["queueDelivery"].clone(), fact["cancelable"].clone()))
        .collect()
}

#[test]
fn a_prompt_written_into_a_claude_turn_is_canceled_when_claude_returns_it() {
    let ledger = ledger();
    let turn = running(&ledger);
    let written = prompt(&ledger, "written", AgentChatPromptDisposition::Queue, &[]);
    assert_eq!(
        join(&ledger, &turn),
        Some(written.message.message_id.clone())
    );
    ledger
        .begin_agent_chat_prompt_launch(&written.message.message_id, DAEMON, EPOCH)
        .unwrap();
    let receipt = cancel(&ledger, &written.message.message_id).unwrap();
    assert_eq!(
        cancel(&ledger, &written.message.message_id).unwrap(),
        receipt
    );
    assert!(facts(&ledger, "promptCanceled").is_empty());
    let removing = facts(&ledger, "promptCancelRequested");
    assert_eq!(removing.len(), 1);
    assert_eq!(removing[0]["messageId"], json!(written.message.message_id));
    assert_eq!(removing[0]["receiptId"], json!(receipt.receipt_id.0));
    assert_eq!(removing[0]["turnId"], json!(turn.message.turn_id));
    assert_eq!(
        ledger
            .canceled_steered_agent_chat_prompts(&turn.run_id)
            .unwrap(),
        std::slice::from_ref(&written.message.message_id)
    );
    stop(&ledger, "stop-after-remove");
    ledger
        .release_agent_chat_prompt_unstarted_launch(&written.message.message_id, DAEMON, EPOCH)
        .unwrap();
    assert_eq!(facts(&ledger, "promptCanceled").len(), 1);
    assert!(withdrawn(&ledger).is_empty());
    assert!(
        ledger
            .canceled_steered_agent_chat_prompts(&turn.run_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ledger
            .find_turn(&written.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Cancelled
    );
}

#[test]
fn a_prompt_written_into_a_codex_turn_is_not_cancelable_and_says_so_when_queued() {
    let ledger = ledger_for(AgentChatProvider::Codex);
    let turn = running(&ledger);
    let written = prompt(&ledger, "written", AgentChatPromptDisposition::Queue, &[]);
    assert_eq!(
        queued_cancelable(&ledger),
        [(json!("joinTurn"), json!(false))]
    );
    assert_eq!(
        join(&ledger, &turn),
        Some(written.message.message_id.clone())
    );
    assert!(matches!(
        cancel(&ledger, &written.message.message_id),
        Err(gent_ports::LedgerError::Rejected(
            gent_types::AgentChatRejection::QueuedPromptNotCancelable
        ))
    ));
}

#[test]
fn claude_and_next_turn_prompts_are_queued_as_cancelable() {
    let ledger = ledger();
    prompt(&ledger, "idle", AgentChatPromptDisposition::Queue, &[]);
    let claude = ledger_for(AgentChatProvider::Claude);
    running(&claude);
    prompt(&claude, "joining", AgentChatPromptDisposition::Queue, &[]);
    assert_eq!(
        queued_cancelable(&ledger),
        [(json!("nextTurn"), json!(true))]
    );
    assert_eq!(
        queued_cancelable(&claude),
        [(json!("joinTurn"), json!(true))]
    );
}
