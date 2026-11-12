use gent_drivers::codex_control::{CodexControlDecision, CodexControlRequest};
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatMode, AgentChatRunId, HostEpoch,
    ReceiptId, WorkspaceRecord,
};

use super::{host, save};
use crate::codex_prompt_lifecycle_tests::{Runner, selection};

#[test]
fn ask_mode_declines_command_and_file_change_approvals_without_asking() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    let mut ask = selection();
    ask.mode = AgentChatMode::Ask;
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: ask,
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    save(&ledger, &conversation_id, "prompt-1", "first");
    let runner = Runner::default();
    let mut host = host(&ledger, runner.clone());
    let approval = |key: &str, method: &str, tool: &str| {
        CodexRunnerEffect::ControlRequest(CodexControlRequest {
            request_id: serde_json::json!(key),
            request_key: key.into(),
            method: method.into(),
            tool_use_id: format!("call-{key}"),
            tool_name: tool.into(),
            input: None,
        })
    };
    host.tick().unwrap();
    runner.state.lock().unwrap().effects.push_back(vec![
        CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "thread-1".into(),
        }),
        approval("0", "item/commandExecution/requestApproval", "Bash"),
        approval("1", "item/fileChange/requestApproval", "apply_patch"),
    ]);
    host.tick().unwrap();
    host.tick().unwrap();
    assert!(
        ledger
            .pending_permission(&conversation_id, &AgentChatRunId("run-a".into()))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runner.state.lock().unwrap().controls,
        [
            ("0".to_owned(), CodexControlDecision::Deny),
            ("1".to_owned(), CodexControlDecision::Deny),
        ]
    );
}
