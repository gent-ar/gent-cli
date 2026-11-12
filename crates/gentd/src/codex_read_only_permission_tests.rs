use gent_drivers::codex_control::{CodexControlDecision, CodexControlRequest};
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatMode, AgentChatRunId, HostEpoch,
    PermissionMode, ReceiptId, WorkspaceRecord,
};

use super::{host, save};
use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, selection};

fn with_two_approvals(
    mode: AgentChatMode,
) -> (
    SqliteLedger,
    Runner,
    ApprovedCodexHost<SqliteLedger, Runner, Resolver>,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    let mut chosen = selection();
    chosen.mode = mode;
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: chosen,
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    ledger
        .save_policy(&gent_types::PolicyRecord {
            policy_id: "policy-a".into(),
            workspace_id: "workspace-a".into(),
            scope: gent_types::PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::Bypass,
            containment: gent_types::Containment::Unsandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        })
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
    (ledger, runner, host)
}

fn pending_key(ledger: &SqliteLedger) -> Option<String> {
    ledger
        .pending_permission(
            &AgentChatConversationId("conversation-a".into()),
            &AgentChatRunId("run-a".into()),
        )
        .unwrap()
        .map(|request| request.binding.decision_id.0)
}

#[test]
fn plan_mode_declines_command_and_file_change_approvals_without_asking() {
    let (ledger, runner, _host) = with_two_approvals(AgentChatMode::Plan);
    assert_eq!(pending_key(&ledger), None);
    assert_eq!(
        runner.state.lock().unwrap().controls,
        [
            ("0".to_owned(), CodexControlDecision::Deny),
            ("1".to_owned(), CodexControlDecision::Deny),
        ]
    );
}

#[test]
fn ask_mode_asks_for_every_command_and_file_change_even_under_bypass() {
    let (ledger, runner, mut host) = with_two_approvals(AgentChatMode::Ask);
    for (key, decision) in [
        ("0", CodexControlDecision::Allow),
        ("1", CodexControlDecision::Deny),
    ] {
        assert_eq!(pending_key(&ledger).as_deref(), Some(key));
        let request = ledger
            .pending_permission(
                &AgentChatConversationId("conversation-a".into()),
                &AgentChatRunId("run-a".into()),
            )
            .unwrap()
            .unwrap();
        ledger.settle_pending_permission(&request.binding).unwrap();
        host.respond_permission("run-a", key, decision, None)
            .unwrap();
        host.tick().unwrap();
    }
    assert_eq!(
        runner.state.lock().unwrap().controls,
        [
            ("0".to_owned(), CodexControlDecision::Allow),
            ("1".to_owned(), CodexControlDecision::Deny),
        ]
    );
}
