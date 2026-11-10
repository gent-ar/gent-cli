use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch,
    PermissionMode, PolicyScope, ReceiptId, WorkspaceRecord,
};

use super::{Resolver, Runner, compatibility, profile, prompt, ready_reply};
use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome;
use crate::public_driver_runtime::PublicDriversRuntime;

fn write_request() -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::PermissionRequest(gent_drivers::claude_control::ClaudePermissionRequest {
        request_id: "permission-p".into(),
        tool_use_id: "tool-p".into(),
        tool_name: "write_file".into(),
        operation_preview: Some(
            serde_json::json!({"file_path":"/workspace-p/main.rs", "content":"hello"}),
        ),
        child_id: None,
    })
}

#[test]
fn a_running_turn_keeps_its_launch_policy_and_the_next_turn_relaunches_under_the_saved_one() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-p".into());
    let run_id = AgentChatRunId("run-p".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
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
    prompt(&ledger, &conversation_id, "p1");
    let runner = Runner::default();
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-session".into(),
        }),
        write_request(),
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
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-p".into(), HostEpoch(1), 1, None);
    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(ClaudePromptDispatchOutcome::Started { .. })
    ));
    let launched = ledger
        .current_policy("workspace-p", PolicyScope::ProviderPermissions)
        .unwrap()
        .unwrap();
    assert_eq!(launched.mode, PermissionMode::AskEveryTime);
    let mut bypass = launched.clone();
    bypass.policy_id = "bypass-saved-mid-turn".into();
    bypass.revision += 1;
    bypass.mode = PermissionMode::Bypass;
    ledger.save_provider_permission_policy(&bypass).unwrap();
    host.tick().unwrap();
    let pending = ledger
        .pending_permission(&conversation_id, &run_id)
        .unwrap()
        .expect("the launch-bound Ask policy still asks during the running turn");
    assert_eq!(pending.binding.policy_revision, launched.revision);
    assert!(runner.0.lock().unwrap().permission_responses.is_empty());
    host.respond_permission(
        "run-p",
        "permission-p",
        gent_drivers::claude_control::ClaudePermissionBehavior::Allow,
        false,
    )
    .unwrap();
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(ready_reply("done"));
    host.tick().unwrap();
    prompt(&ledger, &conversation_id, "p2");
    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(ClaudePromptDispatchOutcome::Started { .. })
    ));
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![write_request()]);
    host.tick().unwrap();
    let state = runner.0.lock().unwrap();
    assert_eq!((state.starts, state.resumes, state.submits), (1, 1, 0));
    assert_eq!(
        state.permission_responses.len(),
        2,
        "the relaunched turn evaluates the saved Bypass revision"
    );
}
