use gent_ports::{AgentChatPromptLedger, AgentChatQueuedPromptLedger};
use gent_types::Command;
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

fn wake(daemon: &mut FakeCodexDaemon, prompt: &AgentChatPromptSaved) {
    daemon
        .router
        .wake_after_prompt_commit(PromptWake {
            conversation_id: AgentChatConversationId(prompt.message.conversation_id.clone()),
            run_id: prompt.run_id.clone(),
            receipt_id: prompt.receipt.receipt_id.clone(),
            disposition: AgentChatPromptDisposition::Queue,
        })
        .unwrap();
}

fn stop(daemon: &mut FakeCodexDaemon, conversation: &AgentChatConversationId) {
    daemon
        .ledger
        .stop_agent_chat_conversation(
            &Command {
                receipt_id: ReceiptId("stop".into()),
                idempotency_key: "agent-chat-interrupt:stop".into(),
                host_epoch: daemon.epoch,
                kind: "agentChatInterrupt".into(),
                payload: serde_json::json!({"conversationId": conversation.0}),
            },
            conversation,
        )
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
fn a_queued_prompt_lands_inside_the_active_codex_turn_and_is_recalled_after_restart() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "steer", "Remember CODE-OWL and WAIT");
    let queued = queue(&mut daemon, &conversation, "Also remember CODE-FALCON");
    wake(&mut daemon, &queued);

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
        queued.receipt.receipt_id.0.as_str()
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
fn a_queued_prompt_that_loses_the_turn_end_race_runs_once_as_the_next_turn() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "race", "Remember CODE-OWL and LATE");
    let queued = queue(&mut daemon, &conversation, "Remember CODE-FALCON");

    wake(&mut daemon, &queued);

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
fn interrupting_a_turn_that_accepted_a_joined_prompt_requeues_it_exactly_once() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) =
        waiting_turn(&mut daemon, "interrupt", "Remember CODE-OWL and HOLD");
    let queued = queue(&mut daemon, &conversation, "Remember CODE-FALCON");
    wake(&mut daemon, &queued);
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
fn queued_prompts_join_the_active_codex_turn_in_queue_order() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "order", "WAIT TWICE for two");
    let first = queue(&mut daemon, &conversation, "Remember CODE-OWL");
    let second = queue(&mut daemon, &conversation, "Remember CODE-FALCON");
    wake(&mut daemon, &second);
    assert_eq!(daemon.settle(&active), DurableTurnPhase::Completed);

    let steered_ids = daemon
        .requests("turn/steer")
        .iter()
        .map(|request| request["params"]["clientUserMessageId"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        steered_ids,
        [
            first.message.message_id.clone(),
            second.message.message_id.clone()
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
            .all(|fact| fact["turnId"] == active.message.turn_id)
    );
    let recall = daemon.prompt(&conversation, "Which code did I give you?");
    assert_eq!(daemon.settle(&recall), DurableTurnPhase::Completed);
    assert_eq!(daemon.reply(&recall), "recall: CODE-FALCON,CODE-OWL");
}

#[test]
fn stop_withdraws_a_prompt_codex_accepted_but_never_consumed() {
    let mut daemon = FakeCodexDaemon::start();
    let (conversation, active) = waiting_turn(&mut daemon, "stop", "Remember CODE-OWL and HOLD");
    let queued = queue(&mut daemon, &conversation, "Remember CODE-FALCON");
    wake(&mut daemon, &queued);
    daemon.drive_until("an accepted joined prompt", |daemon| {
        !daemon.requests("turn/steer").is_empty()
    });
    stop(&mut daemon, &conversation);
    daemon
        .router
        .interrupt_run(AgentChatProvider::Codex, &queued.run_id.0)
        .unwrap();

    assert_eq!(daemon.settle(&active), DurableTurnPhase::Interrupted);
    daemon.drive_until("the withdrawn prompt", |daemon| {
        daemon.phase(&queued).is_terminal()
    });
    assert_eq!(daemon.phase(&queued), DurableTurnPhase::Cancelled);
    let withdrawn = activities(&daemon, &conversation, "promptWithdrawn");
    assert_eq!(withdrawn.len(), 1);
    assert_eq!(withdrawn[0]["activity"]["text"], "Remember CODE-FALCON");
    assert_eq!(
        inputs(&daemon, "turn/start"),
        ["Remember CODE-OWL and HOLD"]
    );
    assert!(activities(&daemon, &conversation, "promptReleased").is_empty());
}
