use std::collections::BTreeSet;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use gent_drivers::LaunchIntent;
use gent_drivers::buffering::BufferPolicy;
use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::claude_turn_options::ClaudeTurnOptions;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch,
    NormalizedLifecycleSignal, PermissionMode, PolicyRecord, PolicyScope, ReceiptId,
    SandboxResourceGrant, SandboxResourceLimits, SandboxWorkspaceAccess, ToolPhase, TurnPhase,
    WorkspaceRecord,
};

use super::{HELPER_SHA256, PinnedSandboxHelper};

#[test]
#[ignore = "explicit real subscription inference and native sandbox verification"]
fn real_claude_broad_mode_reads_and_writes_through_the_pinned_run_gateway() {
    let runtime = PathBuf::from(
        std::env::var_os("GENT_SANDBOX_SMOKE_RUNTIME_DIR").expect("runtime directory"),
    );
    let probe = std::fs::canonicalize(
        std::env::var_os("GENT_SANDBOX_SMOKE_TEMP_DIR").expect("outside-repository temp directory"),
    )
    .unwrap();
    let model =
        std::env::var("GENT_SANDBOX_SMOKE_MODEL").expect("explicit verified model selection");
    let helper = PinnedSandboxHelper::load(&runtime.join("gentd.exe"), HELPER_SHA256).unwrap();
    let executable = PathBuf::from(
        std::env::var_os("GENT_SANDBOX_SMOKE_CLAUDE_EXE")
            .expect("verified native Claude executable"),
    );
    let lock =
        gent_drivers::lock::capture("claude", &executable, "2.1.291", "native-claude-2.1.291")
            .unwrap();
    let root = tempfile::Builder::new()
        .prefix("contained-claude-")
        .tempdir_in(&probe)
        .unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace = std::fs::canonicalize(workspace).unwrap();
    let marker = format!("GENT-CONTAINED-{}", uuid::Uuid::new_v4().simple());
    std::fs::write(workspace.join("marker.txt"), &marker).unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model,
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Agent,
    };
    let conversation = format!("conversation-{}", uuid::Uuid::new_v4().simple());
    let run = format!("run-{}", uuid::Uuid::new_v4().simple());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation.clone()),
                run_id: AgentChatRunId(run.clone()),
                selection: selection.clone(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: workspace.to_string_lossy().into_owned(),
            },
        )
        .unwrap();
    ledger
        .save_policy(&PolicyRecord {
            policy_id: "bypass-policy".into(),
            workspace_id: "workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::Bypass,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        })
        .unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let permission_binding = gent_types::SandboxPermissionBinding::new(
        conversation.clone(),
        run.clone(),
        ledger
            .current_policy("workspace", PolicyScope::ProviderPermissions)
            .unwrap()
            .unwrap(),
        HostEpoch(1),
    )
    .unwrap();
    let lease = crate::provider_egress::ProviderGatewayLease::claude(
        ledger,
        coordinator,
        &permission_binding,
        lock.clone(),
        &probe,
    )
    .unwrap();
    let (resources, shell) = reviewed_resources(&helper, &lock);
    let launcher = lease
        .launcher_with_shell(
            helper.launcher(1024 * 1024),
            SandboxResourceLimits {
                max_processes: 16,
                max_memory_bytes: 2 * 1024 * 1024 * 1024,
                max_cpu_time_ms: 120_000,
            },
            resources,
            Some(shell),
        )
        .unwrap();
    let mut runner = ClaudeStreamRunner::new(
        launcher,
        BufferPolicy::new(64, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner
        .prepare_permission_binding(permission_binding)
        .unwrap();
    let options =
        ClaudeTurnOptions::from_selection_with_permissions(&selection, PermissionMode::Bypass)
            .unwrap()
            .with_conversation_config(None, false, Some(4), vec![]);
    runner.start(ClaudeRunStart { run_id: run.clone(), lock, prompt:
        "Use Read to read marker.txt, then use Write to create proof.txt with the exact same content. Do not use a shell. Finally reply with the marker only.".into(),
        content: vec![], turn_options: options, goal: None, fresh_context: None,
        intent: LaunchIntent::Start, workspace_root: workspace.clone(), workspace_access: SandboxWorkspaceAccess::ReadWrite, mcp_config: None }).unwrap();
    assert!(
        runner
            .sandbox_evidence(&run)
            .unwrap()
            .network_peer()
            .is_some()
    );
    let (output, tools, completed) = drive(&mut runner, &run);
    assert!(completed, "contained Claude turn did not finish");
    assert!(
        output.contains(&marker),
        "provider response did not contain workspace marker"
    );
    assert!(
        tools.contains("Read") && tools.contains("Write"),
        "full native read/write tools were not observed"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("proof.txt")).unwrap(),
        marker
    );
    runner.release(&run).unwrap();
}

fn reviewed_resources(
    helper: &PinnedSandboxHelper,
    lock: &gent_types::RunVersionLock,
) -> (Vec<SandboxResourceGrant>, gent_types::SandboxShellRuntime) {
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    let mut resources = vec![
        SandboxResourceGrant::RuntimeReadOnly(helper.runtime_root().unwrap().to_path_buf()),
        SandboxResourceGrant::RuntimeReadOnly(PathBuf::from(&lock.canonical_path)),
        SandboxResourceGrant::RuntimeReadOnly(
            std::fs::canonicalize(system.join("System32").join("mswsock.dll")).unwrap(),
        ),
    ];
    let runtime_files: Vec<PathBuf> = serde_json::from_slice(
        &std::fs::read(
            std::env::var_os("GENT_SANDBOX_SMOKE_SYSTEM_FILES")
                .expect("reviewed system runtime file list"),
        )
        .unwrap(),
    )
    .unwrap();
    for path in runtime_files {
        resources.push(SandboxResourceGrant::RuntimeReadOnly(
            std::fs::canonicalize(path).unwrap(),
        ));
    }
    let shell_root = std::fs::canonicalize(
        std::env::var_os("GENT_SANDBOX_SMOKE_GIT_ROOT").expect("reviewed Git runtime root"),
    )
    .unwrap();
    let shell_lock = gent_drivers::lock::capture(
        "git-bash",
        &shell_root.join("usr/bin/bash.exe"),
        "5.2.15",
        "native-git-bash-5.2",
    )
    .unwrap();
    let shell = gent_types::SandboxShellRuntime::new(shell_lock).unwrap();
    resources.push(SandboxResourceGrant::RuntimeReadOnly(shell_root));
    (resources, shell)
}

fn drive<L, P>(runner: &mut ClaudeStreamRunner<L, P>, run: &str) -> (String, BTreeSet<String>, bool)
where
    L: gent_drivers::ProcessLauncher<Process = P>,
    P: gent_drivers::ProviderProcess,
{
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut output = String::new();
    let mut tools = BTreeSet::new();
    let mut completed = false;
    while Instant::now() < deadline && !completed {
        let Some(effects) = runner.poll(run).unwrap() else {
            thread::sleep(Duration::from_millis(10));
            continue;
        };
        for effect in effects {
            match effect {
                ClaudeRunnerEffect::Fact(PublicWireFact::Event(
                    gent_types::NormalizedProviderEvent::Output { text, .. },
                )) => output.push_str(&text),
                ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::ToolActivity { activity },
                )) if activity.phase == ToolPhase::Completed => {
                    tools.insert(activity.tool_name);
                }
                ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::RootPhase {
                        phase: TurnPhase::Ready,
                    },
                )) => completed = true,
                ClaudeRunnerEffect::PermissionRequest(_) => {
                    panic!("bypass worker unexpectedly requested a permission callback")
                }
                ClaudeRunnerEffect::Exited { code } => {
                    panic!("contained worker exited before completion: {code:?}")
                }
                _ => {}
            }
        }
    }
    (output, tools, completed)
}
