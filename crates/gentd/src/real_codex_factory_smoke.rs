use gent_drivers::SystemLauncher;
use gent_drivers::{
    buffering::BufferPolicy,
    codex_runner::{CodexAppServerRunner, CodexRunStart},
    codex_session::{CodexSessionConfig, CodexTurnOptions},
};
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::*;
use std::path::{Path, PathBuf};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Inputs {
    qa_data: PathBuf,
    installation_snapshot: PathBuf,
    node: PathBuf,
    authority: PathBuf,
    root_keys: Vec<String>,
    daemon: PathBuf,
    probe_root: PathBuf,
    model: String,
}

#[test]
#[ignore = "explicit one-turn real Codex subscription and native sandbox verification"]
fn real_codex_single_turn_uses_reviewed_policy_and_prepared_factory() {
    exercise(PermissionMode::Bypass, false);
}

#[test]
#[ignore = "explicit one-turn real Autonomous command and edit in owned workspace"]
fn real_codex_autonomous_command_and_edit_use_prepared_factory() {
    exercise(PermissionMode::Autonomous, true);
}

#[path = "real_codex_action_observer.rs"]
mod action_observer;
#[path = "real_codex_smoke_checks.rs"]
mod smoke_checks;
#[path = "real_codex_smoke_controls.rs"]
mod smoke_controls;

#[test]
#[ignore = "explicit one-turn real Bypass command and edit in owned workspace"]
fn real_codex_bypass_command_and_edit_use_prepared_factory() {
    exercise(PermissionMode::Bypass, true);
}

fn exercise(permission: PermissionMode, command_edit: bool) {
    let path = std::env::var_os("GENT_REAL_CODEX_SMOKE_INPUTS").expect("explicit QA inputs");
    let input: Inputs = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_ne!(input.model, "default");
    assert_eq!(
        std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
        std::fs::canonicalize(&input.daemon).unwrap(),
        "normal factory must resolve this actual executable's runtime"
    );
    let installations = SqliteLedger::open(&input.installation_snapshot).unwrap();
    let node =
        crate::node_runtime_lock::AppNodeRuntimeLock::capture(Some(input.node), &input.qa_data)
            .unwrap();
    let release = crate::standalone_authority_release::StandaloneAuthorityRelease::configured(
        input.authority,
        &input.root_keys,
        node,
    )
    .unwrap();
    let executables = crate::provider_executables::ProviderExecutables::standalone(
        None,
        None,
        &input.qa_data,
        installations,
        Some(release),
    );
    let crate::private_provider_readiness::PrivateProviderReadiness::Ready(lock) =
        executables.installed_readiness(AgentChatProvider::Codex)
    else {
        panic!("real approved managed Codex is not Ready");
    };
    let probe = std::fs::canonicalize(input.probe_root).unwrap();
    let root = tempfile::Builder::new()
        .prefix("codex-one-turn-")
        .tempdir_in(&probe)
        .unwrap()
        .keep();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace = std::fs::canonicalize(workspace).unwrap();
    let marker = format!("GENT-SANDBOX-{}", uuid::Uuid::new_v4().simple());
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: input.model,
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    };
    let conversation = format!("conversation-{}", uuid::Uuid::new_v4().simple());
    let run = format!("run-{}", uuid::Uuid::new_v4().simple());
    let ledger = owned_ledger(&conversation, &run, &selection, &workspace);
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let broad = PolicyRecord {
        policy_id: format!("smoke-{permission:?}"),
        workspace_id: "smoke-workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 2,
        mode: permission,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    let response = crate::permission_policy_api::exchange(
        &coordinator,
        gent_protocol::PermissionPolicyFrame::Save {
            request_id: "factory-smoke".into(),
            policy: broad.clone(),
            bypass_consent: permission == PermissionMode::Bypass,
            expected_provider: Some(AgentChatProvider::Codex),
        },
    )
    .unwrap();
    assert!(
        matches!(response,gent_protocol::PermissionPolicyFrame::Saved{policy,..} if policy==broad)
    );
    assert_eq!(
        ledger
            .current_policy("smoke-workspace", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(broad.clone())
    );
    let binding =
        SandboxPermissionBinding::new(conversation.clone(), run.clone(), broad, HostEpoch(1))
            .unwrap();
    let launcher = crate::sandbox_provider_launcher::SandboxProviderLauncher::new(
        SystemLauncher::new(1024 * 1024),
        ledger.clone(),
        coordinator,
        root.join("state"),
        1024 * 1024,
    )
    .with_managed_providers(executables);
    let mut runner = CodexAppServerRunner::new(
        launcher,
        BufferPolicy::new(64, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner.prepare_permission_binding(binding.clone()).unwrap();
    let options = CodexTurnOptions::from_selection_with_permissions(
        &selection,
        Some(&workspace.to_string_lossy()),
        permission,
    )
    .unwrap();
    runner
        .start(CodexRunStart {
            run_id: run.clone(),
            lock,
            session: CodexSessionConfig {
                working_directory: Some(workspace.to_string_lossy().into_owned()),
                resume_thread_id: None,
                turn_options: options,
                mcp_servers: None,
            },
            workspace_root: workspace.clone(),
            workspace_access: SandboxWorkspaceAccess::ReadWrite,
            prompt: if command_edit {
            format!("This is a planned test in an empty owned QA workspace. Use apply_patch to create proof.txt containing exactly {marker} followed by one LF, UTF-8. Then execute one local shell command in this workspace to print proof.txt. Do not access any other paths, network, environment variables or settings. Do not request escalation or additional permissions and do not retry blocked operations. Reply with exactly {marker} only after both file change and command succeeded; otherwise report the actual blocker.")
        } else {
            format!("Reply with exactly {marker}. Do not use tools or access any files.")
        },
            goal: None,
            attachments: vec![],
            interrupted_reply: None,
        })
        .unwrap();
    assert!(
        runner
            .sandbox_evidence(&run)
            .unwrap()
            .network_peer()
            .is_some()
    );
    let turn = smoke_checks::drive(&mut runner, &ledger, &binding, &run);
    let cleanup = runner.terminate(&run);
    println!(
        "real-codex conversation={conversation} run={run} completed={} failure={} cleanup={}",
        turn.completed,
        turn.failure,
        cleanup.is_ok()
    );
    cleanup.unwrap();
    drop(runner);
    smoke_checks::verify(
        &ledger,
        &binding,
        &root,
        &workspace,
        &marker,
        command_edit,
        &turn,
    );
}

pub(crate) fn owned_ledger(
    conversation: &str,
    run: &str,
    selection: &AgentChatSelection,
    workspace: &Path,
) -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-smoke".into()),
                idempotency_key: "create-smoke".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation.into()),
                run_id: AgentChatRunId(run.into()),
                selection: selection.clone(),
            },
            &WorkspaceRecord {
                workspace_id: "smoke-workspace".into(),
                canonical_path: workspace.to_string_lossy().into_owned(),
            },
        )
        .unwrap();
    ledger
        .save_policy(&PolicyRecord {
            policy_id: "smoke-ask".into(),
            workspace_id: "smoke-workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::AskEveryTime,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        })
        .unwrap();
    ledger
}
