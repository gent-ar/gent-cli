use gent_ports::{
    AgentChatConversationConfigLedger, AgentChatReadLedger, AgentChatWorkspaceLedger,
    ConversationLinkLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationConfigRecord, AgentChatConversationCreate, AgentChatConversationId,
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId, AgentChatSelection,
    Containment, ConversationLink, HostEpoch, ReceiptId, WorkspaceRecord,
};

fn conversation(ledger: &SqliteLedger, conversation_id: &str) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("receipt-{conversation_id}")),
                idempotency_key: format!("key-{conversation_id}"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation_id.into()),
                run_id: AgentChatRunId(format!("run-{conversation_id}")),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "sonnet".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
}

fn link(ledger: &SqliteLedger, parent: &str, child: &str) {
    ledger
        .record_conversation_link(
            &ConversationLink {
                parent_conversation_id: parent.into(),
                child_conversation_id: child.into(),
                label: "worker".into(),
                created_run_id: format!("run-{parent}"),
                created_turn_id: format!("turn-{parent}"),
                created_at_unix_seconds: 1,
            },
            &[],
            &format!("link-{child}"),
        )
        .unwrap();
}

fn containment(ledger: &SqliteLedger, conversation_id: &str) -> Containment {
    ledger
        .read_agent_chat_detail(conversation_id)
        .unwrap()
        .summary
        .containment
}

#[test]
fn a_linked_child_inherits_its_sandboxed_parents_containment() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "parent");
    ledger
        .save_conversation_config(&AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("parent".into()),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: vec![],
            containment: Containment::Sandboxed,
        })
        .unwrap();
    conversation(&ledger, "child");
    link(&ledger, "parent", "child");
    assert_eq!(containment(&ledger, "child"), Containment::Sandboxed);
    let config = ledger
        .current_conversation_config("child")
        .unwrap()
        .unwrap();
    assert_eq!(config.containment, Containment::Sandboxed);
    assert_eq!(config.revision, 1);
}

#[test]
fn a_linked_child_of_an_unsandboxed_parent_keeps_the_default_containment() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "parent");
    conversation(&ledger, "child");
    link(&ledger, "parent", "child");
    assert_eq!(containment(&ledger, "child"), Containment::Unsandboxed);
    assert!(
        ledger
            .current_conversation_config("child")
            .unwrap()
            .is_none()
    );
}
