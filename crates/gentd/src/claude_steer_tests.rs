use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatQueuedPromptLedger, PendingPermissionLedger,
};
use gent_types::{
    AgentChatConversationId,
    AgentChatPromptDisposition::{Queue, Send},
    AgentChatPromptSaved, AgentChatProvider, DurableTurnPhase, NormalizedTranscriptKind, ReceiptId,
};
use serde_json::Value;

use super::fake_cli::FakeClaudeDaemon;
use crate::agent_chat_api::{PromptCommitWake, PromptWake};

fn steer(daemon: &mut FakeClaudeDaemon, prompt: &AgentChatPromptSaved) {
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
            disposition: Queue,
        })
        .unwrap();
}

fn written(daemon: &FakeClaudeDaemon) -> bool {
    !daemon
        .ledger
        .has_pending_agent_chat_prompt_dispatch(AgentChatProvider::Claude)
        .unwrap()
}

fn running(daemon: &FakeClaudeDaemon, prompt: &AgentChatPromptSaved) -> bool {
    daemon
        .transcript(prompt)
        .iter()
        .any(|event| event.kind != NormalizedTranscriptKind::UserMessage)
}

fn reply(daemon: &FakeClaudeDaemon, prompt: &AgentChatPromptSaved) -> String {
    daemon
        .transcript(prompt)
        .into_iter()
        .filter(|event| {
            event.kind == NormalizedTranscriptKind::AssistantMessage && !event.is_partial
        })
        .map(|event| event.text)
        .collect()
}

fn activities(
    daemon: &FakeClaudeDaemon,
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

fn settle(daemon: &mut FakeClaudeDaemon, prompt: &AgentChatPromptSaved) -> DurableTurnPhase {
    daemon.drive_until("a settled Claude turn", |daemon| {
        daemon.phase(prompt).is_terminal()
    });
    daemon.phase(prompt)
}

#[test]
fn a_steer_is_taken_up_by_the_running_claude_turn_and_survives_a_resume() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, _) = daemon.conversation("steer");
    let active = daemon.prompt(&conversation, "Remember CODE-OWL then SLOW", Send);
    daemon.drive_until("a running turn", |daemon| running(daemon, &active));
    let first = daemon.prompt(&conversation, "Remember CODE-FALCON", Queue);
    let second = daemon.prompt(&conversation, "Remember CODE-HERON", Queue);
    steer(&mut daemon, &first);
    steer(&mut daemon, &second);

    assert_eq!(settle(&mut daemon, &active), DurableTurnPhase::Completed);
    assert_eq!(daemon.phase(&first), DurableTurnPhase::Completed);
    assert_eq!(daemon.phase(&second), DurableTurnPhase::Completed);
    assert_eq!(daemon.launches().len(), 1);
    assert_eq!(
        reply(&daemon, &active),
        "recall: CODE-FALCON,CODE-HERON,CODE-OWL"
    );
    let steered = activities(&daemon, &conversation, "promptSteered");
    let ids = steered
        .iter()
        .map(|fact| fact["activity"]["messageId"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            first.message.message_id.clone(),
            second.message.message_id.clone()
        ]
    );
    assert!(
        steered
            .iter()
            .all(|fact| fact["turnId"] == active.message.turn_id)
    );
    assert!(activities(&daemon, &conversation, "promptReleased").is_empty());
    assert!(reply(&daemon, &first).is_empty());

    let (other, _) = daemon.conversation("other");
    let unrelated = daemon.prompt(&other, "hello", Send);
    assert_eq!(settle(&mut daemon, &unrelated), DurableTurnPhase::Completed);
    let recall = daemon.prompt(&conversation, "Which code did I give you?", Send);
    assert_eq!(settle(&mut daemon, &recall), DurableTurnPhase::Completed);
    assert_eq!(
        reply(&daemon, &recall),
        "recall: CODE-FALCON,CODE-HERON,CODE-OWL"
    );
    assert!(
        daemon
            .launches()
            .last()
            .unwrap()
            .iter()
            .any(|argument| argument == "--resume")
    );
}

#[test]
fn a_steer_claude_takes_up_after_the_turn_result_runs_once_as_its_own_turn() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, _) = daemon.conversation("race");
    let active = daemon.prompt(&conversation, "Remember CODE-OWL then RACE", Send);
    daemon.drive_until("a running turn", |daemon| running(daemon, &active));
    let queued = daemon.prompt(&conversation, "Remember CODE-FALCON", Queue);

    steer(&mut daemon, &queued);

    assert_eq!(settle(&mut daemon, &active), DurableTurnPhase::Completed);
    assert_eq!(settle(&mut daemon, &queued), DurableTurnPhase::Completed);
    assert_eq!(reply(&daemon, &active), "recall: CODE-OWL");
    assert_eq!(reply(&daemon, &queued), "recall: CODE-FALCON,CODE-OWL");
    assert_eq!(daemon.launches().len(), 1);
    assert!(activities(&daemon, &conversation, "promptSteered").is_empty());
    assert_eq!(
        activities(&daemon, &conversation, "promptReleased").len(),
        1
    );
}

#[test]
fn interrupting_a_turn_before_claude_takes_up_the_steer_runs_it_once_next() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.conversation("interrupt");
    let active = daemon.prompt(&conversation, "Remember CODE-OWL then HOLD", Send);
    daemon.drive_until("a running turn", |daemon| running(daemon, &active));
    let queued = daemon.prompt(&conversation, "Remember CODE-FALCON", Queue);
    steer(&mut daemon, &queued);
    daemon.drive_until("the steer written to Claude", written);

    daemon
        .router
        .interrupt_run(AgentChatProvider::Claude, &run.0)
        .unwrap();
    assert_eq!(settle(&mut daemon, &active), DurableTurnPhase::Interrupted);
    assert_eq!(settle(&mut daemon, &queued), DurableTurnPhase::Completed);
    assert_eq!(reply(&daemon, &queued), "recall: CODE-FALCON,CODE-OWL");
    assert_eq!(daemon.launches().len(), 2);
    assert!(activities(&daemon, &conversation, "promptSteered").is_empty());
    assert_eq!(
        activities(&daemon, &conversation, "promptReleased").len(),
        1
    );
}

#[test]
fn a_steer_sent_while_a_permission_is_pending_joins_that_turn() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, run) = daemon.conversation("permission");
    let active = daemon.prompt(&conversation, "Remember CODE-OWL then PERMISSION", Send);
    daemon.drive_until("durable pending permission", |daemon| {
        daemon
            .ledger
            .pending_permission(&conversation, &run)
            .unwrap()
            .is_some()
    });
    let queued = daemon.prompt(&conversation, "Remember CODE-FALCON", Queue);

    steer(&mut daemon, &queued);
    daemon.drive_until("the steer taken up", |daemon| {
        daemon.phase(&queued).is_terminal()
    });
    daemon
        .router
        .interrupt_run(AgentChatProvider::Claude, &run.0)
        .unwrap();

    assert_eq!(settle(&mut daemon, &active), DurableTurnPhase::Interrupted);
    let steered = activities(&daemon, &conversation, "promptSteered");
    assert_eq!(steered.len(), 1);
    assert_eq!(steered[0]["turnId"], active.message.turn_id);
    let recall = daemon.prompt(&conversation, "Which code did I give you?", Send);
    assert_eq!(settle(&mut daemon, &recall), DurableTurnPhase::Completed);
    assert_eq!(reply(&daemon, &recall), "recall: CODE-FALCON,CODE-OWL");
}
