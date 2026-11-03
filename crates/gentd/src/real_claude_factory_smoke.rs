use gent_drivers::SystemLauncher;
use gent_drivers::{
    LaunchIntent,
    buffering::BufferPolicy,
    claude_runner::{ClaudeRunStart, ClaudeStreamRunner},
    claude_turn_options::ClaudeTurnOptions,
};
use gent_ports::PolicyLedger;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::*;
use std::path::PathBuf;

#[path = "real_claude_smoke_turn.rs"]
mod smoke_turn;

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
#[ignore = "explicit one-turn real Claude subscription and native sandbox verification"]
fn real_claude_bypass_uses_reviewed_policy_and_prepared_factory() {
    run(PermissionMode::Bypass);
}

#[test]
#[ignore = "explicit one-turn Claude subscription and native sandbox verification"]
fn real_claude_autonomous_uses_reviewed_policy_and_prepared_factory() {
    run(PermissionMode::Autonomous);
}

fn run(mode: PermissionMode) {
    let path = std::env::var_os("GENT_REAL_CLAUDE_SMOKE_INPUTS").expect("explicit QA inputs");
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
        executables.installed_readiness(AgentChatProvider::Claude)
    else {
        panic!("real approved managed Claude is not Ready");
    };
    let probe = std::fs::canonicalize(input.probe_root).unwrap();
    let root = tempfile::Builder::new()
        .prefix("claude-one-turn-")
        .tempdir_in(&probe)
        .unwrap()
        .keep();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace = std::fs::canonicalize(workspace).unwrap();
    let marker = format!("GENT-SANDBOX-{}", uuid::Uuid::new_v4().simple());
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: input.model,
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    };
    let conversation = format!("conversation-{}", uuid::Uuid::new_v4().simple());
    let run = format!("run-{}", uuid::Uuid::new_v4().simple());
    let ledger =
        super::real_codex_factory_smoke::owned_ledger(&conversation, &run, &selection, &workspace);
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let broad = PolicyRecord {
        policy_id: "smoke-bypass".into(),
        workspace_id: "smoke-workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 2,
        mode,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    let response = crate::permission_policy_api::exchange(
        &coordinator,
        gent_protocol::PermissionPolicyFrame::Save {
            request_id: "factory-smoke".into(),
            policy: broad.clone(),
            bypass_consent: true,
            expected_provider: Some(AgentChatProvider::Claude),
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
    let mut runner = ClaudeStreamRunner::new(
        launcher,
        BufferPolicy::new(64, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner.prepare_permission_binding(binding.clone()).unwrap();
    let options = ClaudeTurnOptions::from_selection_with_permissions(&selection, mode)
        .unwrap()
        .with_conversation_config(None, false, Some(4), vec![]);
    std::fs::write(workspace.join("marker.txt"), &marker).unwrap();
    runner.start(ClaudeRunStart {
        run_id: run.clone(), lock,
        prompt: "Read marker.txt using Read, then create proof.txt with identical content using Write. Do not run a shell. Reply with the marker only.".into(),
        content: vec![], turn_options: options, goal: None, fresh_context: None,
        intent: LaunchIntent::Start, workspace_root: workspace.clone(),
        workspace_access: SandboxWorkspaceAccess::ReadWrite, mcp_config: None,
    }).unwrap();
    assert!(
        runner
            .sandbox_evidence(&run)
            .unwrap()
            .network_peer()
            .is_some()
    );
    let turn = smoke_turn::Session {
        runner: &mut runner,
        ledger: &ledger,
        binding: &binding,
        run: &run,
        conversation: &conversation,
    }
    .drive();
    let cleanup = runner.release(&run);
    println!(
        "real-claude run={run} completed={} failure={} cleanup={}",
        turn.completed,
        turn.failure,
        cleanup.is_ok()
    );
    cleanup.unwrap();
    drop(runner);
    assert!(
        !turn.failure && turn.completed,
        "single turn failed; no retry"
    );
    assert!(turn.output.contains(&marker));
    assert!(turn.tools.contains("Read") && turn.tools.contains("Write"));
    assert_eq!(
        std::fs::read_to_string(workspace.join("proof.txt")).unwrap(),
        marker
    );
    std::fs::remove_dir_all(&root).unwrap();
}
