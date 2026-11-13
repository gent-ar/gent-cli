use gent_ports::{
    AgentChatConversationConfigLedger, AgentChatProjectionLedger, AgentChatSelectionLedger,
    AgentChatWorkspaceLedger,
};
use gent_types::*;

use crate::SqliteLedger;

fn selection(mode: AgentChatMode) -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "fixture".into(),
        effort: AgentChatEffort::Medium,
        mode,
    }
}

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
                selection: selection(AgentChatMode::Agent),
            },
            &WorkspaceRecord {
                workspace_id: "summary-fixture".into(),
                canonical_path: "/workspace/summary-fixture".into(),
            },
        )
        .unwrap();
    ledger
}

fn summaries(ledger: &SqliteLedger, after: u64) -> Vec<(u64, serde_json::Value)> {
    ledger
        .agent_chat_projection_page(&AgentChatConversationId("conversation".into()), after, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "summary")
        .map(|event| (event.cursor, event.payload))
        .collect()
}

fn switch(ledger: &SqliteLedger, key: &str) -> AgentChatSelectionSwitched {
    ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId(key.into()),
            idempotency_key: key.into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation".into()),
            parent_run_id: AgentChatRunId("run".into()),
            run_id: AgentChatRunId("switched".into()),
            selection: selection(AgentChatMode::Ask),
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap()
}

#[test]
fn a_selection_switch_appends_exactly_one_summary_event_naming_the_new_run() {
    let ledger = ledger();
    assert!(summaries(&ledger, 0).is_empty());
    switch(&ledger, "switch");
    switch(&ledger, "switch");
    let appended = summaries(&ledger, 0);
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].1, serde_json::json!({"runId": "switched"}));
    assert!(summaries(&ledger, appended[0].0).is_empty());
}

#[test]
fn a_conversation_config_save_appends_a_summary_event() {
    let ledger = ledger();
    ledger
        .save_conversation_config(&AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation".into()),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: Vec::new(),
            containment: Containment::Sandboxed,
        })
        .unwrap();
    assert_eq!(
        summaries(&ledger, 0)
            .into_iter()
            .map(|(_, payload)| payload)
            .collect::<Vec<_>>(),
        vec![serde_json::json!({"runId": "run"})]
    );
}
