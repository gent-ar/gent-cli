use gent_ports::{AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger};
use gent_types::*;

use crate::SqliteLedger;

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "fixture".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "held-recovery-fixture".into(),
                canonical_path: "/workspace/held-recovery-fixture".into(),
            },
        )
        .unwrap();
    ledger
}

fn save(ledger: &SqliteLedger, id: usize) -> AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{id}")),
            receipt_id: ReceiptId(format!("receipt-{id}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: format!("original {id}"),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap()
}

#[test]
fn bounded_pages_keep_original_receipts_messages_and_monotonic_cursor_without_duplicates() {
    let ledger = ledger();
    let mut originals = (0..130).map(|id| save(&ledger, id)).collect::<Vec<_>>();
    originals.sort_by(|a, b| a.message.message_id.cmp(&b.message.message_id));
    let first = ledger
        .read_awaiting_agent_chat_prompts(None, "", 128)
        .unwrap();
    assert_eq!(first.len(), 128);
    let last = &first.last().unwrap().message.message_id;
    let second = ledger
        .read_awaiting_agent_chat_prompts(None, last, 128)
        .unwrap();
    assert_eq!(second.len(), 2);
    let recovered = first.into_iter().chain(second).collect::<Vec<_>>();
    for (before, after) in originals.iter().zip(&recovered) {
        assert_eq!(before.receipt, after.receipt);
        assert_eq!(before.message, after.message);
    }
    assert!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 129)
            .is_err()
    );
    assert!(
        ledger
            .read_awaiting_agent_chat_prompts(Some("wrong-run"), "", 128)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn provisioning_and_dispatch_ambiguity_are_never_read_as_undelivered_readiness_holds() {
    let ledger = ledger();
    let original = save(&ledger, 1);
    for state in [
        "provisioning",
        "pending",
        "claimed",
        "launching",
        "started",
        "unprovable",
        "settled",
    ] {
        ledger
            .lock()
            .unwrap()
            .execute("UPDATE agent_chat_prompt_dispatches SET state=?1", [state])
            .unwrap();
        assert!(
            ledger
                .read_awaiting_agent_chat_prompts(None, "", 128)
                .unwrap()
                .is_empty(),
            "{state}"
        );
    }
    ledger
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state='awaiting_readiness'",
            [],
        )
        .unwrap();
    assert_eq!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 128)
            .unwrap()[0]
            .message,
        original.message
    );
    ledger
        .lock()
        .unwrap()
        .execute("UPDATE turns SET phase='completed'", [])
        .unwrap();
    assert!(
        ledger
            .read_awaiting_agent_chat_prompts(None, "", 128)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn every_targeted_steer_disposition_remains_excluded_even_when_readiness_is_held() {
    let ledger = ledger();
    let original = save(&ledger, 1);
    ledger
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO agent_chat_steer_targets (message_id,receipt_id,run_id,turn_id,state)
         VALUES (?1,'steer','run',?2,'requested')",
            rusqlite::params![original.message.message_id, original.message.turn_id],
        )
        .unwrap();
    for state in ["requested", "claimed", "delivered", "rejected"] {
        ledger
            .lock()
            .unwrap()
            .execute("UPDATE agent_chat_steer_targets SET state=?1", [state])
            .unwrap();
        assert!(
            ledger
                .read_awaiting_agent_chat_prompts(None, "", 128)
                .unwrap()
                .is_empty(),
            "{state}"
        );
    }
}

#[test]
fn inconsistent_receipt_message_turn_bindings_cannot_be_reconstructed() {
    let ledger = ledger();
    let first = save(&ledger, 1);
    let second = save(&ledger, 2);
    ledger
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO turns (turn_id,conversation_id,run_id,sequence,phase)
             VALUES ('mismatched-turn','conversation','run',999,'active')",
            [],
        )
        .unwrap();
    ledger
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_chat_prompt_receipts SET turn_id=?1 WHERE message_id=?2",
            rusqlite::params!["mismatched-turn", first.message.message_id],
        )
        .unwrap();
    let page = ledger
        .read_awaiting_agent_chat_prompts(None, "", 128)
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].message, second.message);
}
