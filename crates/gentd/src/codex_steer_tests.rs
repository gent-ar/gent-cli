use gent_ports::{AgentChatPromptLedger, AgentChatQueuedPromptLedger};
use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatPromptSaved, AgentChatProvider, AgentChatRejection, AgentChatRequestId,
    DurableTurnPhase, ReceiptId,
};
use serde_json::Value;

use super::fake_cli::FakeCodexDaemon;
use crate::agent_chat_api::{PromptCommitWake, PromptWake};

fn queue(
    daemon: &mut FakeCodexDaemon,
    conversation: &AgentChatConversationId,
    text: &str,
) -> AgentChatPromptSaved {
    let saved = daemon
        .ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("queued-{text}")),
            receipt_id: ReceiptId(format!("queued-receipt-{text}")),
            host_epoch: daemon.epoch,
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Queue,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: text.into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&daemon.ledger, &saved);
    saved
}

fn steer(daemon: &mut FakeCodexDaemon, prompt: &AgentChatPromptSaved) {
    let conversation = AgentChatConversationId(prompt.message.conversation_id.clone());
    let (receipt, run_id) = daemon
        .ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId(format!("steer-{}", prompt.message.message_id)),
            daemon.epoch,
            &conversation,
            &prompt.message.message_id,
        )
        .unwrap();
    daemon
        .router
        .wake_after_prompt_commit(PromptWake {
            conversation_id: conversation,
            run_id,
            receipt_id: receipt.receipt_id,
            disposition: AgentChatPromptDisposition::Queue,
        })
        .unwrap();
}

fn activities(
    daemon: &FakeCodexDaemon,
    conversation: &AgentChatConversationId,
    kind: &str,
) -> Vec<Value> {
    daemon
        .projection(conversation)
        .into_iter()
        .filter(|event| event.payload["activity"]["type"] == kind)
        .map(|event| event.payload)
        .collect()
}

fn inputs(daemon: &FakeCodexDaemon, method: &str) -> Vec<String> {
    daemon
        .requests(method)
        .iter()
        .map(|request| {
            request["params"]["input"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn waiting_turn(
    daemon: &mut FakeCodexDaemon,
    key: &str,
    text: &str,
) -> (AgentChatConversationId, AgentChatPromptSaved) {
    let (conversation, _) = daemon.conversation(key);
    let active = daemon.prompt(&conversation, text);
    daemon.drive_until("streamed output", |daemon| {
        !daemon.reply(&active).is_empty()
    });
    (conversation, active)
}

#[test]
fn a_steer_lands_inside_the_active_codex_turn_and_is_recalled_after_restart() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "steer", "Remember CODE-OWL and WAIT");
    let queued = queue(&mut daemon, &conversation, "Also remember CODE-FALCON");
    daemon.drive_until("queued behind the active turn", |_| true);
    assert!(daemon.requests("turn/steer").is_empty());

    steer(&mut daemon, &queued);

    assert_eq!(daemon.settle(&active), DurableTurnPhase::Completed);
    let steers = daemon.requests("turn/steer");
    assert_eq!(steers.len(), 1);
    assert_eq!(
        steers[0]["params"]["clientUserMessageId"],
        queued.message.message_id
    );
    assert_eq!(
        inputs(&daemon, "turn/start"),
        ["Remember CODE-OWL and WAIT"]
    );
    assert_eq!(daemon.phase(&queued), DurableTurnPhase::Completed);
    let steered = activities(&daemon, &conversation, "promptSteered");
    assert_eq!(steered.len(), 1);
    assert_eq!(steered[0]["turnId"], active.message.turn_id);
    assert_eq!(
        steered[0]["activity"]["messageId"],
        queued.message.message_id
    );
    assert_eq!(
        steered[0]["activity"]["receiptId"],
        format!("steer-{}", queued.message.message_id)
    );
    assert!(activities(&daemon, &conversation, "promptReleased").is_empty());
    let projection = daemon.projection(&conversation);
    let cursor = |predicate: &dyn Fn(&Value) -> bool| {
        projection
            .iter()
            .find(|event| predicate(&event.payload))
            .unwrap()
            .cursor
    };
    let output = cursor(&|payload| {
        payload["turnId"] == active.message.turn_id && payload["kind"] == "assistantMessage"
    });
    let steer_cursor = cursor(&|payload| payload["activity"]["type"] == "promptSteered");
    let terminal = cursor(&|payload| {
        payload["activity"]["type"] == "terminal" && payload["turnId"] == active.message.turn_id
    });
    assert!(output < steer_cursor && steer_cursor < terminal);
    assert!(matches!(
        daemon.ledger.cancel_queued_agent_chat_prompt(
            &ReceiptId("late-cancel".into()),
            daemon.epoch,
            &conversation,
            &queued.message.message_id
        ),
        Err(gent_ports::LedgerError::Rejected(
            AgentChatRejection::QueuedPromptNotCancelable
        ))
    ));

    let recall = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-FALCON,CODE-OWL");
    daemon.restart();
    let restored = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&restored), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&restored), "recall: CODE-FALCON,CODE-OWL");
    assert_eq!(daemon.requests("thread/resume").len(), 1);
}

#[test]
fn a_steer_that_loses_the_turn_end_race_runs_once_as_the_next_turn() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "race", "Remember CODE-OWL and LATE");
    let queued = queue(&mut daemon, &conversation, "Remember CODE-FALCON");

    steer(&mut daemon, &queued);

    assert_eq!(daemon.settle(&active), DurableTurnPhase::Completed);
    assert_eq!(daemon.settle(&queued), DurableTurnPhase::Completed);
    assert_eq!(daemon.requests("turn/steer").len(), 1);
    assert_eq!(
        inputs(&daemon, "turn/start"),
        ["Remember CODE-OWL and LATE", "Remember CODE-FALCON"]
    );
    assert_eq!(daemon.reply(&queued), "recall: CODE-OWL");
    assert!(activities(&daemon, &conversation, "promptSteered").is_empty());
    assert_eq!(
        activities(&daemon, &conversation, "promptReleased").len(),
        1
    );
    let recall = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-FALCON,CODE-OWL");
}

#[test]
fn interrupting_a_turn_that_accepted_a_steer_requeues_the_prompt_exactly_once() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) =
        waiting_turn(&mut daemon, "interrupt", "Remember CODE-OWL and HOLD");
    let queued = queue(&mut daemon, &conversation, "Remember CODE-FALCON");
    steer(&mut daemon, &queued);
    daemon.drive_until("an accepted steer", |daemon| {
        !daemon.requests("turn/steer").is_empty()
    });

    daemon
        .router
        .interrupt_run(AgentChatProvider::Codex, &queued.run_id.0)
        .unwrap();

    assert_eq!(daemon.settle(&active), DurableTurnPhase::Interrupted);
    assert_eq!(daemon.settle(&queued), DurableTurnPhase::Completed);
    assert_eq!(
        inputs(&daemon, "turn/start"),
        ["Remember CODE-OWL and HOLD", "Remember CODE-FALCON"]
    );
    assert_eq!(daemon.reply(&queued), "recall: CODE-OWL");
    assert!(activities(&daemon, &conversation, "promptSteered").is_empty());
    assert_eq!(
        activities(&daemon, &conversation, "promptReleased").len(),
        1
    );
    let recall = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-FALCON,CODE-OWL");
}

#[test]
fn steers_keep_queue_order_and_never_jump_an_unsteered_prompt() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "order", "WAIT for the first");
    let first = queue(
        &mut daemon,
        &conversation,
        "Remember CODE-OWL and WAIT TWICE",
    );
    let second = queue(&mut daemon, &conversation, "Remember CODE-FALCON");
    let third = queue(&mut daemon, &conversation, "Remember CODE-HERON");
    steer(&mut daemon, &second);
    steer(&mut daemon, &third);
    daemon.drive_until("steers held behind an unsteered prompt", |_| true);
    assert!(daemon.requests("turn/steer").is_empty());

    daemon
        .router
        .interrupt_run(AgentChatProvider::Codex, &active.run_id.0)
        .unwrap();
    assert_eq!(daemon.settle(&active), DurableTurnPhase::Interrupted);
    assert_eq!(daemon.settle(&first), DurableTurnPhase::Completed);

    let steers = daemon.requests("turn/steer");
    let steered_ids = steers
        .iter()
        .map(|request| request["params"]["clientUserMessageId"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        steered_ids,
        [
            second.message.message_id.clone(),
            third.message.message_id.clone()
        ]
    );
    let facts = activities(&daemon, &conversation, "promptSteered");
    assert_eq!(
        facts
            .iter()
            .map(|fact| fact["activity"]["messageId"].clone())
            .collect::<Vec<_>>(),
        steered_ids
    );
    assert!(
        facts
            .iter()
            .all(|fact| fact["turnId"] == first.message.turn_id)
    );
    let recall = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(
        daemon.reply(&recall),
        "recall: CODE-FALCON,CODE-HERON,CODE-OWL"
    );
}
