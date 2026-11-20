use gent_ports::{AgentChatProjectionLedger, AgentChatWorkspaceLedger};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, ConversationActivityFact,
    ConversationActivityScope, HostEpoch, ReceiptId, TurnPhase, WorkspaceFileChange,
    WorkspaceRecord,
};

use super::SqliteLedger;

fn conversation(ledger: &SqliteLedger) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
}

fn terminal(ledger: &SqliteLedger, turn_id: &str) {
    let connection = ledger.lock().unwrap();
    crate::sqlite::agent_chat_queue_activity::append_activity(
        &connection,
        format!("terminal:{turn_id}"),
        ReceiptId(format!("terminal:{turn_id}")),
        "agentChatQueueActivity",
        ConversationActivityFact::Terminal {
            scope: ConversationActivityScope {
                conversation_id: "conversation".into(),
                run_id: "run".into(),
                turn_id: turn_id.into(),
                host_epoch: HostEpoch(1),
                cursor: 0,
            },
            phase: TurnPhase::Ready,
            cause: None,
        },
    )
    .unwrap();
}

#[test]
fn each_terminal_after_the_cursor_carries_its_workspace_and_baseline_and_records_one_fact() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger);
    let head = ledger.agent_chat_projection_head().unwrap();
    assert!(!ledger.has_workspace_baseline("conversation").unwrap());
    ledger
        .record_workspace_baseline("conversation", "a".repeat(40).as_str(), HostEpoch(1))
        .unwrap();
    ledger
        .record_workspace_baseline("conversation", "b".repeat(40).as_str(), HostEpoch(1))
        .unwrap();
    terminal(&ledger, "turn-1");

    let turns = ledger.workspace_change_turns(head, 16).unwrap();
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    assert_eq!(
        (
            turn.turn_id.as_str(),
            turn.workspace_path.as_str(),
            turn.baseline_tree.clone()
        ),
        ("turn-1", "/workspace", Some("a".repeat(40)))
    );
    let files = vec![WorkspaceFileChange {
        path: "notes.md".into(),
        added: 3,
        removed: 0,
    }];
    ledger
        .record_workspace_changes(turn, files.clone(), false)
        .unwrap();
    ledger
        .record_workspace_changes(turn, files.clone(), false)
        .unwrap();

    let recorded: Vec<_> = ledger
        .agent_chat_projection_page(&AgentChatConversationId("conversation".into()), 0, 64)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.payload["activity"]["type"] == "workspaceChanges")
        .collect();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].payload["activity"]["files"],
        serde_json::to_value(&files).unwrap()
    );
    assert!(
        ledger
            .workspace_change_turns(turn.projection_cursor, 16)
            .unwrap()
            .is_empty()
    );
}
