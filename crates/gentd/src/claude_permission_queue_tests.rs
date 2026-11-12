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

fn read(id: &str) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::PermissionRequest(ClaudePermissionRequest {
        request_id: id.into(),
        tool_use_id: format!("tool-{id}"),
        tool_name: "write_file".into(),
        operation_preview: Some(serde_json::json!({"file_path": format!("/workspace-q/{id}.rs")})),
        child_id: None,
    })
}

fn host_with_two_requests_in(
    mode: AgentChatMode,
    permission: gent_types::PermissionMode,
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
        })
        .unwrap();
    prompt(&ledger, &conversation_id, "q1");
    let runner = Runner::default();
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-session".into(),
        }),
        read("first"),
        read("second"),
    ]);
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
fn a_turn_that_ends_with_a_queued_request_denies_it_and_leaves_nothing_pending() {
    let (ledger, runner, mut host) = host_with_two_requests();
    assert_eq!(pending_id(&ledger).as_deref(), Some("first"));
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
        [("second", ClaudePermissionBehavior::Deny)]
    );
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

const READ_ONLY: gent_types::PermissionDenialReason =
    gent_types::PermissionDenialReason::ReadOnlyMode;

#[test]
fn plan_mode_refuses_file_changes_with_the_read_only_reason() {
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
            ("first", ClaudePermissionBehavior::Refuse(READ_ONLY)),
            ("second", ClaudePermissionBehavior::Refuse(READ_ONLY)),
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
