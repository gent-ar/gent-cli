use gent_drivers::claude_control::ClaudePermissionBehavior;
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, DecisionOutcome,
    HostEpoch, PermissionDecisionResponse, PermissionDecisionResponseKind, PermissionMode,
    PolicyRecord, PolicyScope, ReceiptId, WorkspaceRecord,
};

use super::{Resolver, Runner, compatibility, profile, prompt, ready_reply};
use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome;
use crate::public_driver_runtime::PublicDriversRuntime;

type Host = ApprovedClaudeHost<SqliteLedger, Runner, Resolver>;

fn request(id: &str, tool_name: &str, input: serde_json::Value) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::PermissionRequest(gent_drivers::claude_control::ClaudePermissionRequest {
        request_id: id.into(),
        tool_use_id: format!("tool-{id}"),
        tool_name: tool_name.into(),
        operation_preview: Some(input),
        child_id: None,
    })
}

fn write(id: &str) -> ClaudeRunnerEffect {
    request(
        id,
        "write_file",
        serde_json::json!({"file_path":"/workspace-p/main.rs", "content":"hello"}),
    )
}

fn command(id: &str) -> ClaudeRunnerEffect {
    request(id, "Bash", serde_json::json!({"command":"cargo test"}))
}

fn conversation() -> AgentChatConversationId {
    AgentChatConversationId("conversation-p".into())
}

fn run() -> AgentChatRunId {
    AgentChatRunId("run-p".into())
}

fn started(launch: PermissionMode, first: Vec<ClaudeRunnerEffect>) -> (SqliteLedger, Runner, Host) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation(),
                run_id: run(),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-p".into(),
                canonical_path: "/workspace-p".into(),
            },
        )
        .unwrap();
    ledger
        .save_policy(&PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "workspace-p".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: launch,
            allowed_tools: Vec::new(),
            allowed_categories: Vec::new(),
            allowed_commands: Vec::new(),
            containment: gent_types::Containment::Unsandboxed,
        })
        .unwrap();
    prompt(&ledger, &conversation(), "p1");
    let runner = Runner::default();
    let mut effects = vec![ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
        provider_session_id: "private-session".into(),
    })];
    effects.extend(first);
    runner.0.lock().unwrap().effects.push_back(effects);
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
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-p".into(), HostEpoch(1), 1, None);
    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(ClaudePromptDispatchOutcome::Started { .. })
    ));
    (ledger, runner, host)
}

fn current(ledger: &SqliteLedger) -> PolicyRecord {
    ledger
        .current_policy("workspace-p", PolicyScope::ProviderPermissions)
        .unwrap()
        .unwrap()
}

fn save_mode(ledger: &SqliteLedger, mode: PermissionMode) {
    let mut next = current(ledger);
    next.revision += 1;
    next.policy_id = format!("policy-{}", next.revision);
    next.mode = mode;
    ledger.save_provider_permission_policy(&next).unwrap();
}

fn push(runner: &Runner, effects: Vec<ClaudeRunnerEffect>) {
    runner.0.lock().unwrap().effects.push_back(effects);
}

fn answers(runner: &Runner) -> Vec<(String, ClaudePermissionBehavior)> {
    runner
        .0
        .lock()
        .unwrap()
        .permission_responses
        .iter()
        .map(|(_, request, behavior, _)| (request.clone(), *behavior))
        .collect()
}

fn waiting(ledger: &SqliteLedger) -> Option<gent_types::PermissionDecisionRequest> {
    ledger.pending_permission(&conversation(), &run()).unwrap()
}

fn waiting_id(ledger: &SqliteLedger) -> Option<String> {
    waiting(ledger).map(|pending| pending.binding.decision_id.0)
}

#[test]
fn switching_to_bypass_mid_turn_answers_the_waiting_card_and_every_later_request() {
    let (ledger, runner, mut host) = started(PermissionMode::AskEveryTime, vec![write("write-1")]);
    host.tick().unwrap();
    assert_eq!(waiting_id(&ledger).as_deref(), Some("write-1"));
    save_mode(&ledger, PermissionMode::Bypass);
    host.tick().unwrap();
    assert_eq!(waiting_id(&ledger), None);
    push(&runner, vec![command("command-2")]);
    host.tick().unwrap();
    assert_eq!(waiting_id(&ledger), None);
    assert_eq!(
        answers(&runner),
        [
            ("write-1".to_owned(), ClaudePermissionBehavior::Allow),
            ("command-2".to_owned(), ClaudePermissionBehavior::Allow),
        ]
    );
    let state = runner.0.lock().unwrap();
    assert_eq!((state.starts, state.resumes), (1, 0));
}

#[test]
fn a_category_grant_mid_turn_allows_the_next_request_of_that_category_in_the_same_run() {
    let (ledger, runner, mut host) =
        started(PermissionMode::AskEveryTime, vec![command("command-1")]);
    host.tick().unwrap();
    let pending = waiting(&ledger).expect("Ask every time asks for a command");
    let response = PermissionDecisionResponse {
        binding: pending.binding.clone(),
        response: PermissionDecisionResponseKind::ApproveCategory,
        input: None,
        command_pattern: None,
    };
    let granted =
        crate::agent_chat_permission_api::grant::transition(&current(&ledger), &pending, &response)
            .unwrap();
    host.respond_permission("run-p", "command-1", ClaudePermissionBehavior::Allow, false)
        .unwrap();
    ledger
        .settle_delivered_permission(&pending.binding, granted.as_ref(), DecisionOutcome::Allowed)
        .unwrap();
    push(&runner, vec![command("command-2")]);
    host.tick().unwrap();
    assert_eq!(waiting_id(&ledger), None);
    assert_eq!(
        answers(&runner),
        [
            ("command-1".to_owned(), ClaudePermissionBehavior::Allow),
            ("command-2".to_owned(), ClaudePermissionBehavior::Allow),
        ]
    );
}

#[test]
fn tightening_mid_turn_asks_at_once_and_the_next_prompt_relaunches_under_the_saved_policy() {
    let (ledger, runner, mut host) = started(PermissionMode::Bypass, vec![write("write-1")]);
    host.tick().unwrap();
    assert_eq!(waiting_id(&ledger), None);
    save_mode(&ledger, PermissionMode::AskEveryTime);
    push(&runner, vec![write("write-2")]);
    host.tick().unwrap();
    let pending = waiting(&ledger).expect("Ask every time asks again in the same turn");
    assert_eq!(pending.binding.decision_id.0, "write-2");
    host.respond_permission("run-p", "write-2", ClaudePermissionBehavior::Allow, false)
        .unwrap();
    ledger
        .settle_delivered_permission(&pending.binding, None, DecisionOutcome::Allowed)
        .unwrap();
    push(&runner, ready_reply("done"));
    host.tick().unwrap();
    prompt(&ledger, &conversation(), "p2");
    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(ClaudePromptDispatchOutcome::Started { .. })
    ));
    let state = runner.0.lock().unwrap();
    assert_eq!((state.starts, state.resumes), (1, 1));
    assert_eq!(state.permission_responses.len(), 2);
}
