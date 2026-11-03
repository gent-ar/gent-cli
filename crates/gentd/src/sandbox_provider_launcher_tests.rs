use super::*;
use gent_ports::AgentChatWorkspaceLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch,
    PermissionMode, PolicyRecord, ReceiptId, WorkspaceRecord,
};

fn prepared() -> (
    tempfile::TempDir,
    SandboxProviderLauncher,
    SandboxPermissionBinding,
) {
    prepared_for(AgentChatProvider::Claude)
}
fn prepared_for(
    provider: AgentChatProvider,
) -> (
    tempfile::TempDir,
    SandboxProviderLauncher,
    SandboxPermissionBinding,
) {
    let root = tempfile::tempdir().unwrap();
    let workspace = std::fs::canonicalize(root.path()).unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider,
                    model: "test".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: workspace.to_string_lossy().into_owned(),
            },
        )
        .unwrap();
    let policy = PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::AskEveryTime,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    ledger.save_policy(&policy).unwrap();
    gent_ports::AgentChatConversationConfigLedger::save_conversation_config(
        &ledger,
        &gent_types::AgentChatConversationConfigRecord {
            conversation_id: gent_types::AgentChatConversationId("conversation".into()),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: vec![],
            containment: gent_types::Containment::Sandboxed,
        },
    )
    .unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let launcher = SandboxProviderLauncher::new(
        SystemLauncher::new(1024),
        ledger,
        coordinator,
        root.path().join("state"),
        1024,
    );
    let binding =
        SandboxPermissionBinding::new("conversation".into(), "run".into(), policy, HostEpoch(1))
            .unwrap();
    (root, launcher, binding)
}

#[test]
fn routed_factory_requires_exact_current_policy_and_open_epoch() {
    let (_root, launcher, binding) = prepared();
    launcher.current(&binding).unwrap();
    let mut policy = binding.policy().clone();
    policy.revision += 1;
    policy.policy_id = "policy-revision-2".into();
    launcher.ledger.save_policy(&policy).unwrap();
    assert!(launcher.validate_permission_binding(&binding).is_err());
    let fresh =
        SandboxPermissionBinding::new("conversation".into(), "run".into(), policy, HostEpoch(1))
            .unwrap();
    launcher.current(&fresh).unwrap();
    launcher.ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(launcher.validate_permission_binding(&fresh).is_err());
}

#[test]
fn routed_factory_never_turns_a_broad_policy_into_a_host_launch() {
    let (_root, launcher, binding) = prepared();
    let mut policy = binding.policy().clone();
    policy.revision += 1;
    policy.policy_id = "policy-revision-2".into();
    policy.mode = PermissionMode::Bypass;
    launcher.ledger.save_policy(&policy).unwrap();
    let broad =
        SandboxPermissionBinding::new("conversation".into(), "run".into(), policy, HostEpoch(1))
            .unwrap();
    assert!(launcher.validate_permission_binding(&broad).is_err());
    let launch = ProviderLaunch {
        permission_binding: Some(broad),
        run_id: Some("run".into()),
        provider: "claude".into(),
        executable: "never-executed".into(),
        arguments: vec![],
        intent: gent_drivers::LaunchIntent::Start,
        workspace_root: None,
        workspace_access: SandboxWorkspaceAccess::ReadWrite,
        lock: gent_types::RunVersionLock {
            provider: "claude".into(),
            canonical_path: "never-executed".into(),
            file_identity: "not-a-file".into(),
            digest_sha256: "a".repeat(64),
            version: "test".into(),
            compatibility_entry: "test".into(),
        },
    };
    assert!(launcher.launch(&launch).is_err());
    let mut unbound = launch;
    unbound.permission_binding = None;
    assert!(launcher.launch(&unbound).is_err());
}

#[test]
fn windows_claude_and_codex_ordinary_modes_require_native_preparation_without_host_fallback() {
    for provider in [AgentChatProvider::Claude, AgentChatProvider::Codex] {
        let (_root, launcher, binding) = prepared_for(provider);
        assert!(launcher.validate_permission_binding(&binding).is_err());
    }
}
