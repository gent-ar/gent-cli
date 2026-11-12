use super::*;
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch,
    PermissionMode, PolicyRecord, PolicyScope, ReceiptId, WorkspaceRecord,
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
fn a_policy_saved_after_claim_applies_to_the_next_prompt_without_failing_this_launch() {
    let (_root, launcher, claimed) = prepared();
    let mut changed = claimed.policy().clone();
    changed.revision += 1;
    changed.policy_id = "policy-revision-2".into();
    changed.mode = PermissionMode::Bypass;
    launcher.ledger.save_policy(&changed).unwrap();
    gent_ports::AgentChatConversationConfigLedger::save_conversation_config(
        &launcher.ledger,
        &gent_types::AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation".into()),
            revision: 2,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: vec![],
            containment: gent_types::Containment::Unsandboxed,
        },
    )
    .unwrap();
    assert_eq!(
        launcher.current(&claimed).unwrap(),
        AgentChatProvider::Claude
    );
    let next = crate::permission_workspace::policy_for_conversation(
        &launcher.ledger,
        "conversation",
        "workspace",
    )
    .unwrap();
    assert_eq!(
        next,
        changed.for_conversation(gent_types::Containment::Unsandboxed)
    );
    let next =
        SandboxPermissionBinding::new("conversation".into(), "run".into(), next, HostEpoch(1))
            .unwrap();
    launcher.current(&next).unwrap();
    launcher.ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(launcher.current(&claimed).is_err());
    assert!(launcher.current(&next).is_err());
}

#[test]
fn a_selection_switch_keeps_the_launch_authority_of_the_superseded_run() {
    let (_root, launcher, binding) = prepared();
    gent_ports::AgentChatSelectionLedger::switch_agent_chat_selection(
        &launcher.ledger,
        &gent_types::AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation".into()),
            parent_run_id: AgentChatRunId("run".into()),
            run_id: AgentChatRunId("next".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6-luna".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        },
    )
    .unwrap();
    assert_eq!(
        launcher.current(&binding).unwrap(),
        AgentChatProvider::Claude
    );
    let absent = SandboxPermissionBinding::new(
        "conversation".into(),
        "absent".into(),
        binding.policy().clone(),
        HostEpoch(1),
    )
    .unwrap();
    assert!(launcher.current(&absent).is_err());
    launcher.ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(launcher.current(&binding).is_err());
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
