//! A real, bounded, credential-free kernel probe. Its scratch-only binding is
//! never a user run permission grant and cannot be published to a user lease.
use gent_drivers::{
    ProcessLauncher, ProviderProcess, SandboxedLauncher,
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    supervisor::ProviderLaunch,
};
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, Containment, HostEpoch, PermissionMode,
    PolicyRecord, PolicyScope, ReceiptId, RunVersionLock, SandboxCpuSemantics, SandboxLaunchPolicy,
    SandboxNetworkPolicy, SandboxPermissionBinding, SandboxPrivateState, SandboxResourceGrant,
    SandboxResourceLimits, SandboxWorkspaceAccess, SandboxedLaunchRequest, WorkspaceRecord,
};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

#[path = "linux_provider_probe_slot.rs"]
mod slot;
pub(crate) trait ProbeInputs: Send + Sync {
    fn observe(&self, _event: serde_json::Value) {}
    fn recheck(&self) -> Result<(), String>;
    fn executable(&self) -> &RunVersionLock;
    fn helper_lock(&self) -> &RunVersionLock;
    fn delegated_parent(&self) -> &Path;
    fn fingerprint(&self) -> &str;
    fn grants(&self) -> Vec<SandboxResourceGrant>;
    fn sdk_version(&self) -> &str;
}
struct Authority {
    _root: tempfile::TempDir,
    _ledger: gent_store::SqliteLedger,
    _resources: Arc<dyn ProbeInputs>,
}

// A failed launch cannot prove that no helper was created. Retain THIS exact
// scratch/pins/ledger before launch, conservatively block another attempt, and
// never infer settlement from a different/global process queue.
fn held() -> &'static slot::Slot<Authority> {
    static HELD: OnceLock<slot::Slot<Authority>> = OnceLock::new();
    HELD.get_or_init(slot::Slot::new)
}
pub(crate) struct OsProbeWitness {
    manifest: String,
    provider: RunVersionLock,
    parent_identity: (u64, u64),
}
impl OsProbeWitness {
    pub(super) fn matches(&self, resources: &dyn ProbeInputs) -> bool {
        use std::os::unix::fs::MetadataExt;
        self.manifest == resources.fingerprint()
            && self.provider == *resources.executable()
            && resources
                .delegated_parent()
                .metadata()
                .is_ok_and(|m| (m.dev(), m.ino()) == self.parent_identity)
    }
}

pub(super) fn run(resources: Arc<dyn ProbeInputs>) -> Result<OsProbeWitness, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    resources.recheck()?;
    if held().busy() {
        return Err("Previous Linux preflight ownership is unresolved; restart the owned service after settlement".into());
    }
    let root = tempfile::Builder::new()
        .prefix("gent-linux-preflight-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .map_err(|_| "Linux preflight scratch unavailable")?;
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir(&workspace).map_err(|_| "Linux preflight workspace")?;
    std::fs::create_dir(&state).map_err(|_| "Linux preflight state")?;
    let (ledger, run_id, binding) = probe_context(root.path(), &workspace)?;
    let private = SandboxPrivateState::new(run_id.clone(), state)
        .map_err(|_| "Linux preflight private state")?;
    for path in private.directories() {
        std::fs::create_dir(path).map_err(|_| "Linux preflight private directories")?;
    }
    let mut grants = resources.grants();
    grants.push(SandboxResourceGrant::PrivateStateReadWrite(
        private.root().to_owned(),
    ));
    crate::provider_egress::linux_resources::include_helper_image(
        &mut grants,
        resources.helper_lock(),
    )
    .map_err(|_| "Linux preflight helper resource")?;
    let policy = SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 64,
            max_memory_bytes: 1024 * 1024 * 1024,
            max_cpu_time_ms: 10_000,
        },
    )
    .map_err(|_| "Linux preflight limits")?
    .with_resources(grants)
    .with_private_state(private)
    .with_permission_binding(binding.clone())
    .with_cpu_semantics(SandboxCpuSemantics::MonitoredTotal {
        poll_ms: 5,
        wall_ms: 15_000,
    })
    .map_err(|_| "Linux preflight CPU contract")?;
    let expected_request = SandboxedLaunchRequest {
        lock: resources.executable().clone(),
        profile: policy
            .profile_for_workspace(&workspace, SandboxWorkspaceAccess::ReadOnly)
            .map_err(|_| "Linux preflight exact profile")?,
    };
    let launcher = SandboxedLauncher::new(
        policy,
        gent_linux_sandbox::LinuxMxcLauncher {
            delegated_parent: resources.delegated_parent().into(),
            helper_lock: resources.helper_lock().clone(),
            gateway_authority: None,
        },
    );
    let launch = ProviderLaunch {
        provider: "claude".into(),
        executable: Path::new(&resources.executable().canonical_path).into(),
        lock: resources.executable().clone(),
        arguments: vec!["--version".into()],
        intent: gent_drivers::supervisor::LaunchIntent::Start,
        run_id: Some(run_id),
        workspace_root: Some(workspace),
        workspace_access: SandboxWorkspaceAccess::ReadOnly,
        permission_binding: Some(binding),
    };
    let parent = resources
        .delegated_parent()
        .metadata()
        .map_err(|_| "Linux preflight parent changed")?;
    let parent_identity = (parent.dev(), parent.ino());
    let authority = Arc::new(Authority {
        _root: root,
        _ledger: ledger,
        _resources: resources.clone(),
    });
    held().reserve(authority.clone()).map_err(str::to_owned)?;
    let process = launcher.launch(&launch).map_err(
        |_| "Linux kernel probe failed; scratch and resource authority retained conservatively",
    )?;
    let execution_witness = process.probe_execution_witness();
    let probe_run_id = launch.run_id.as_deref().unwrap_or_default();
    let recorded_execution = execution_witness
        .as_ref()
        .is_some_and(|w| w.matches(&expected_request, probe_run_id));
    let mut different_image = expected_request.clone();
    different_image.lock.digest_sha256 = "0".repeat(64);
    let different_run_rejected = execution_witness
        .as_ref()
        .is_some_and(|w| !w.matches(&expected_request, "not-this-probe"));
    let different_image_rejected = execution_witness
        .as_ref()
        .is_some_and(|w| !w.matches(&different_image, probe_run_id));
    let live_at_read = matches!(process.try_exit_code(), Ok(None));
    let witnessed = recorded_execution && different_run_rejected && different_image_rejected;
    resources.observe(serde_json::json!({"phase":"witness","backendRecordedExecution":recorded_execution,"differentRunRejected":different_run_rejected,"differentImageRejected":different_image_rejected,"liveAtCallerReadDiagnosticOnly":live_at_read}));
    let (stdout, exit) = collect_exit(&process)?;
    if !held().release_after_scope_known(&authority) {
        return Err("Linux probe ownership generation changed after settlement".into());
    }
    let expected_version = format!("{} (Claude Code)", resources.sdk_version());
    resources.observe(serde_json::json!({"phase":"settled","scopeKnown":true,"exitCode":exit,"stdoutBytes":stdout.len(),"exactVersionMatched":std::str::from_utf8(&stdout).ok().map(str::trim_end)==Some(expected_version.as_str())}));
    if !witnessed
        || exit != Some(0)
        || std::str::from_utf8(&stdout).ok().map(str::trim_end) != Some(expected_version.as_str())
        || stdout.len() > 4096
    {
        return Err(
            "Linux kernel probe did not establish the exact managed contained version exit".into(),
        );
    }
    let seal = OsProbeWitness {
        manifest: resources.fingerprint().into(),
        provider: resources.executable().clone(),
        parent_identity,
    };
    if seal.manifest != resources.fingerprint() || seal.provider != *resources.executable() {
        return Err("Linux deployment changed during kernel probe".into());
    }
    Ok(seal)
}

fn probe_context(
    root: &Path,
    workspace: &Path,
) -> Result<(gent_store::SqliteLedger, String, SandboxPermissionBinding), String> {
    let ledger = gent_store::SqliteLedger::open(root.join("probe.sqlite"))
        .map_err(|_| "Linux preflight ledger unavailable")?;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let conversation = format!("linux-preflight-conversation-{nonce}");
    let run_id = format!("linux-preflight-run-{nonce}");
    let workspace_id = format!("linux-preflight-workspace-{nonce}");
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(nonce.clone()),
                idempotency_key: nonce.clone(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation.clone()),
                run_id: AgentChatRunId(run_id.clone()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Ask,
                },
            },
            &WorkspaceRecord {
                workspace_id: workspace_id.clone(),
                canonical_path: workspace.to_str().ok_or("Linux preflight path")?.into(),
            },
        )
        .map_err(|_| "Linux preflight durable context")?;
    let policy = PolicyRecord {
        policy_id: format!("linux-preflight-policy-{nonce}"),
        workspace_id,
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::AskEveryTime,
        containment: Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    ledger
        .save_policy(&policy)
        .map_err(|_| "Linux preflight policy")?;
    let binding = SandboxPermissionBinding::new(conversation, run_id.clone(), policy, HostEpoch(1))
        .map_err(|_| "Linux preflight binding")?;
    Ok((ledger, run_id, binding))
}

fn collect_exit<P>(process: &P) -> Result<(Vec<u8>, Option<i32>), String>
where
    P: ProviderProcess + ProcessTreeControl,
{
    let begun = Instant::now();
    let mut stdout = Vec::new();
    let mut exit = None;
    while begun.elapsed() < Duration::from_secs(15) {
        if let Ok(Some(bytes)) = process.next_stdout_chunk() {
            stdout.extend(bytes);
            if stdout.len() > 4096 {
                break;
            }
        }
        match process.try_exit_code() {
            Ok(Some(code)) => {
                exit = Some(code);
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if exit.is_none() {
        let _ = process.signal_tree(ProcessTreeSignal::Kill);
        let stopped = Instant::now();
        while stopped.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(code)) = process.try_exit_code() {
                exit = Some(code);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if exit.is_none() {
        return Err("Linux kernel probe scope settlement is unknown; authority retained".into());
    }
    while stdout.len() <= 4096 {
        match process.next_stdout_chunk() {
            Ok(Some(bytes)) => stdout.extend(bytes),
            _ => break,
        }
    }
    Ok((stdout, exit.flatten()))
}
