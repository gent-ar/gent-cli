use gent_ports::{
    AgentChatArchiveLedger, AgentChatPromptLedger, AgentChatReadLedger, AgentChatWorkspaceLedger,
    ConversationLedger, LedgerError,
};
use gent_types::*;

use crate::SqliteLedger;

fn ledger_with(conversations: &[&str]) -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    for id in conversations {
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId(format!("create-{id}")),
                    idempotency_key: format!("create-{id}"),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId((*id).into()),
                    run_id: AgentChatRunId(format!("run-{id}")),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Codex,
                        model: "fixture".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "archive-fixture".into(),
                    canonical_path: "/workspace/archive-fixture".into(),
                },
            )
            .unwrap();
    }
    ledger
}

fn archive(ledger: &SqliteLedger, receipt: &str, id: &str) -> Result<Receipt, LedgerError> {
    ledger.archive_agent_chat_conversation(
        &ReceiptId(receipt.into()),
        HostEpoch(1),
        &AgentChatConversationId(id.into()),
    )
}

fn listed(ledger: &SqliteLedger) -> Vec<String> {
    ledger
        .list_conversations()
        .unwrap()
        .into_iter()
        .map(|item| item.conversation_id)
        .collect()
}

fn prompt(ledger: &SqliteLedger, id: &str, key: &str) -> Result<AgentChatPromptSaved, LedgerError> {
    ledger.save_agent_chat_prompt(&AgentChatPromptCreate {
        request_id: AgentChatRequestId(format!("request-{id}-{key}")),
        receipt_id: ReceiptId(format!("receipt-{id}-{key}")),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId(id.into()),
        disposition: AgentChatPromptDisposition::Send,
        text: "hello".into(),
        attachment_ids: vec![],
        tool_source_ids: vec![],
    })
}

#[test]
fn an_archived_conversation_leaves_the_list_and_refuses_prompts_but_stays_readable() {
    let ledger = ledger_with(&["kept", "archived"]);
    prompt(&ledger, "archived", "before").unwrap();
    let receipt = archive(&ledger, "archive-1", "archived").unwrap();
    assert_eq!(archive(&ledger, "archive-1", "archived").unwrap(), receipt);
    archive(&ledger, "archive-2", "archived").unwrap();
    assert_eq!(listed(&ledger), vec!["kept".to_owned()]);
    assert!(matches!(
        prompt(&ledger, "archived", "after"),
        Err(LedgerError::Rejected(
            AgentChatRejection::ConversationArchived
        ))
    ));
    prompt(&ledger, "kept", "after").unwrap();
    assert_eq!(
        ledger
            .read_agent_chat_detail("archived")
            .unwrap()
            .current_run_id,
        "run-archived"
    );
    assert!(
        !ledger
            .read_agent_chat_transcript("archived", None, 10)
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn archiving_an_unknown_conversation_is_a_typed_rejection() {
    let ledger = ledger_with(&["kept"]);
    assert!(matches!(
        archive(&ledger, "archive-unknown", "missing"),
        Err(LedgerError::Rejected(
            AgentChatRejection::ConversationNotFound
        ))
    ));
    assert!(archive(&ledger, "archive-unknown", "kept").is_ok());
    assert!(matches!(
        archive(&ledger, "archive-unknown", "missing"),
        Err(LedgerError::Invariant(_))
    ));
}
