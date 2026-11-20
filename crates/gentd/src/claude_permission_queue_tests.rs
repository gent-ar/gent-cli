use gent_drivers::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, ReceiptId,
    WorkspaceRecord,
};

use super::{Resolver, Runner, compatibility, profile, prompt, ready_reply};
use crate::approved_claude_host::ApprovedClaudeHost;
use crate::public_driver_runtime::PublicDriversRuntime;

fn tool(id: &str, tool_name: &str, preview: serde_json::Value) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::PermissionRequest(ClaudePermissionRequest {
        request_id: id.into(),
        tool_use_id: format!("tool-{id}"),
        tool_name: tool_name.into(),
        operation_preview: Some(preview),
        child_id: None,
    })
}

fn read(id: &str) -> ClaudeRunnerEffect {
    tool(
        id,
        "write_file",
        serde_json::json!({"file_path": format!("/workspace-q/{id}.rs")}),
    )
}

fn host_with_two_requests_in(
    mode: AgentChatMode,
    permission: gent_types::PermissionMode,
) -> (
    SqliteLedger,
    Runner,
    ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
) {
    host_with_requests_in(mode, permission, vec![read("first"), read("second")])
}

fn host_with_requests_in(
    mode: AgentChatMode,
    permission: gent_types::PermissionMode,
    requests: Vec<ClaudeRunnerEffect>,
) -> (
    SqliteLedger,
    Runner,
    ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-q".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-q".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-q".into(),
                canonical_path: "/workspace-q".into(),
            },
        )
        .unwrap();
    ledger
        .save_policy(&gent_types::PolicyRecord {
            policy_id: "policy-q".into(),
            workspace_id: "workspace-q".into(),
            scope: gent_types::PolicyScope::ProviderPermissions,
            revision: 1,
            mode: permission,
            containment: gent_types::Containment::Unsandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
            allowed_commands: Vec::new(),
        })
        .unwrap();
    prompt(&ledger, &conversation_id, "q1");
    let runner = Runner::default();
    runner.0.lock().unwrap().effects.push_back(
        std::iter::once(ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-session".into(),
        }))
        .chain(requests)
        .collect(),
    );
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-q".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    host.tick().unwrap();
    (ledger, runner, host)
}

fn pending_id(ledger: &SqliteLedger) -> Option<String> {
    ledger
        .pending_permission(
            &AgentChatConversationId("conversation-q".into()),
            &AgentChatRunId("run-q".into()),
        )
        .unwrap()
        .map(|request| request.binding.decision_id.0)
}

fn answer(
    ledger: &SqliteLedger,
    host: &mut ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
    id: &str,
) {
    let pending = ledger
        .pending_permission(
            &AgentChatConversationId("conversation-q".into()),
            &AgentChatRunId("run-q".into()),
        )
        .unwrap()
        .unwrap();
    ledger.settle_pending_permission(&pending.binding).unwrap();
    host.respond_permission("run-q", id, ClaudePermissionBehavior::Allow, false)
        .unwrap();
}

#[test]
fn parallel_permission_requests_surface_one_at_a_time_and_the_turn_completes() {
    let (ledger, runner, mut host) = host_with_two_requests();
    assert_eq!(pending_id(&ledger).as_deref(), Some("first"));
    answer(&ledger, &mut host, "first");
    host.tick().unwrap();
    assert_eq!(pending_id(&ledger).as_deref(), Some("second"));
    answer(&ledger, &mut host, "second");
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(ready_reply("done"));
    host.tick().unwrap();
    assert_eq!(pending_id(&ledger), None);
    let responses = runner.0.lock().unwrap().permission_responses.clone();
    assert_eq!(
        responses
            .iter()
            .map(|response| (response.1.as_str(), response.2))
            .collect::<Vec<_>>(),
        [
            ("first", ClaudePermissionBehavior::Allow),
            ("second", ClaudePermissionBehavior::Allow),
        ]
    );
}

#[test]
fn a_request_still_queued_when_the_turn_ends_is_asked_not_denied() {
    let (ledger, runner, mut host) = host_with_two_requests();
    assert_eq!(pending_id(&ledger).as_deref(), Some("first"));
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(ready_reply("done"));
    host.tick().unwrap();
    host.tick().unwrap();
    assert_eq!(pending_id(&ledger).as_deref(), Some("second"));
    assert!(runner.0.lock().unwrap().permission_responses.is_empty());
}

fn host_with_two_requests() -> (
    SqliteLedger,
    Runner,
    ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
) {
    host_with_two_requests_in(
        AgentChatMode::Agent,
        gent_types::PermissionMode::AskEveryTime,
    )
}

const PLAN_EDIT: gent_types::PermissionDenialReason =
    gent_types::PermissionDenialReason::PlanModeEdit;

#[test]
fn plan_mode_refuses_file_changes_with_the_plan_edit_reason() {
    let (ledger, runner, _host) =
        host_with_two_requests_in(AgentChatMode::Plan, gent_types::PermissionMode::Bypass);
    assert_eq!(pending_id(&ledger), None);
    let responses = runner.0.lock().unwrap().permission_responses.clone();
    assert_eq!(
        responses
            .iter()
            .map(|response| (response.1.as_str(), response.2))
            .collect::<Vec<_>>(),
        [
            ("first", ClaudePermissionBehavior::Refuse(PLAN_EDIT)),
            ("second", ClaudePermissionBehavior::Refuse(PLAN_EDIT)),
        ]
    );
}

#[test]
fn ask_mode_asks_for_every_file_change_and_allow_or_deny_reach_claude() {
    let (ledger, runner, mut host) =
        host_with_two_requests_in(AgentChatMode::Ask, gent_types::PermissionMode::Bypass);
    assert_eq!(pending_id(&ledger).as_deref(), Some("first"));
    answer(&ledger, &mut host, "first");
    host.tick().unwrap();
    assert_eq!(pending_id(&ledger).as_deref(), Some("second"));
    let pending = ledger
        .pending_permission(
            &AgentChatConversationId("conversation-q".into()),
            &AgentChatRunId("run-q".into()),
        )
        .unwrap()
        .unwrap();
    ledger.settle_pending_permission(&pending.binding).unwrap();
    host.respond_permission("run-q", "second", ClaudePermissionBehavior::Deny, false)
        .unwrap();
    assert_eq!(
        runner
            .0
            .lock()
            .unwrap()
            .permission_responses
            .iter()
            .map(|response| (response.1.as_str(), response.2))
            .collect::<Vec<_>>(),
        [
            ("first", ClaudePermissionBehavior::Allow),
            ("second", ClaudePermissionBehavior::Deny),
        ]
    );
}

#[test]
fn plan_mode_asks_for_commands_refuses_edits_and_never_prompts_for_subagents() {
    let (ledger, runner, mut host) = host_with_requests_in(
        AgentChatMode::Plan,
        gent_types::PermissionMode::AskEveryTime,
        vec![
            read("edit"),
            tool(
                "agent",
                "Agent",
                serde_json::json!({"prompt": "run py -3 --version"}),
            ),
            tool(
                "command",
                "Bash",
                serde_json::json!({"command": "git --version"}),
            ),
        ],
    );
    assert_eq!(pending_id(&ledger).as_deref(), Some("command"));
    answer(&ledger, &mut host, "command");
    assert_eq!(
        runner
            .0
            .lock()
            .unwrap()
            .permission_responses
            .iter()
            .map(|response| (response.1.as_str(), response.2))
            .collect::<Vec<_>>(),
        [
            ("edit", ClaudePermissionBehavior::Refuse(PLAN_EDIT)),
            ("agent", ClaudePermissionBehavior::Allow),
            ("command", ClaudePermissionBehavior::Allow),
        ]
    );
}

#[test]
fn plan_mode_commands_follow_an_unsandboxed_bypass_without_asking() {
    let (ledger, runner, _host) = host_with_requests_in(
        AgentChatMode::Plan,
        gent_types::PermissionMode::Bypass,
        vec![tool(
            "command",
            "PowerShell",
            serde_json::json!({"command": "py -3 --version"}),
        )],
    );
    assert_eq!(pending_id(&ledger), None);
    assert_eq!(
        runner
            .0
            .lock()
            .unwrap()
            .permission_responses
            .iter()
            .map(|response| (response.1.as_str(), response.2))
            .collect::<Vec<_>>(),
        [("command", ClaudePermissionBehavior::Allow)]
    );
}
