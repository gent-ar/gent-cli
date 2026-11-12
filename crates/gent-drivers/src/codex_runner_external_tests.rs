//! Pure wire/ownership controls: these fixtures do not claim native OS containment.
use super::bind_external_process;
use crate::{
    ProcessSandboxEvidence, ProviderProcess,
    codex_session::{CodexSessionConfig, CodexTurnOptions},
    codex_turn::{CodexTurnDriver, CodexTurnEffect},
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    supervisor::{LaunchIntent, ProviderLaunch},
};
use gent_types::*;
use serde_json::{Value, json};
struct Process {
    proof: Option<ProcessSandboxEvidence>,
    state: u8,
}
impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}
impl ProviderProcess for Process {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.proof.as_ref()
    }
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        panic!("binding must not send provider frames")
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        match self.state {
            0 => Ok(None),
            1 => Ok(Some(Some(0))),
            _ => Err(ProcessTreeError::Failed("unknown".into())),
        }
    }
}
fn fixture(
    mode: PermissionMode,
    role: AgentChatMode,
    resume: bool,
) -> (
    tempfile::TempDir,
    ProviderLaunch,
    SandboxedLaunchRequest,
    CodexTurnDriver,
) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().canonicalize().unwrap();
    let binding = SandboxPermissionBinding::new(
        "conversation".into(),
        "run".into(),
        PolicyRecord {
            policy_id: "policy".into(),
            workspace_id: "workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        },
        HostEpoch(1),
    )
    .unwrap();
    let lock = RunVersionLock {
        provider: "codex".into(),
        canonical_path: workspace.join("codex.exe").display().to_string(),
        file_identity: "owned-file".into(),
        digest_sha256: "a".repeat(64),
        version: "1".into(),
        compatibility_entry: "test".into(),
    };
    let access = SandboxWorkspaceAccess::from_mode(role);
    let profile = SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1,
            max_cpu_time_ms: 1,
        },
    )
    .unwrap()
    .with_permission_binding(binding.clone())
    .profile_for_workspace(&workspace, access)
    .unwrap();
    let request = SandboxedLaunchRequest {
        lock: lock.clone(),
        profile,
    };
    let launch = ProviderLaunch {
        permission_binding: Some(binding),
        run_id: Some("run".into()),
        lock,
        provider: "codex".into(),
        executable: request.lock.canonical_path.clone().into(),
        arguments: vec![],
        intent: LaunchIntent::Start,
        workspace_root: Some(workspace.clone()),
        workspace_access: access,
    };
    let options = CodexTurnOptions::from_selection_with_permissions(
        &AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "explicit-test".into(),
            effort: AgentChatEffort::Low,
            mode: role,
        },
        Some(&workspace.display().to_string()),
        mode,
    )
    .unwrap();
    let (turn, _) = CodexTurnDriver::start(
        CodexSessionConfig {
            working_directory: Some(workspace.display().to_string()),
            resume_thread_id: resume.then(|| "thread".into()),
            turn_options: options,
            mcp_servers: None,
        },
        "test",
        None,
    )
    .unwrap();
    (root, launch, request, turn)
}
fn process(request: &SandboxedLaunchRequest) -> Process {
    Process {
        proof: Some(
            ProcessSandboxEvidence::new(
                request,
                &request.lock,
                "run".into(),
                "pid-birth".into(),
                SandboxBackendId::new("test-native".into()).unwrap(),
            )
            .unwrap(),
        ),
        state: 0,
    }
}
fn parameters(turn: &mut CodexTurnDriver) -> Value {
    turn.receive(&serde_json::to_vec(&json!({"id":1,"result":{}})).unwrap())
        .unwrap();
    let effects = turn
        .receive(&serde_json::to_vec(&json!({"id":2,"result":{"thread":{"id":"thread"}}})).unwrap())
        .unwrap();
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Write(frame) => serde_json::from_slice::<Value>(&frame).ok(),
            _ => None,
        })
        .find(|v| v["method"] == "turn/start")
        .unwrap()["params"]
        .clone()
}
#[test]
fn external_policy_requires_matching_live_process_and_preserves_all_mode_approvals() {
    for role in [
        AgentChatMode::Ask,
        AgentChatMode::Plan,
        AgentChatMode::Agent,
    ] {
        for (mode, approval) in [
            (PermissionMode::AskEveryTime, "untrusted"),
            (PermissionMode::AutoAcceptEdits, "untrusted"),
            (PermissionMode::Autonomous, "on-request"),
            (PermissionMode::Bypass, "never"),
        ] {
            for resume in [false, true] {
                let (_root, launch, request, mut turn) = fixture(mode, role, resume);
                bind_external_process(&process(&request), &launch, true, &mut turn).unwrap();
                let p = parameters(&mut turn);
                assert_eq!(
                    p["sandboxPolicy"],
                    json!({"type":"externalSandbox","networkAccess":"enabled"})
                );
                assert_eq!(
                    p["approvalPolicy"],
                    if role == AgentChatMode::Agent {
                        approval
                    } else {
                        "never"
                    }
                );
            }
        }
    }
}
#[test]
fn policy_or_host_launcher_alone_never_selects_external_sandbox() {
    let (_root, launch, _, mut turn) =
        fixture(PermissionMode::AutoAcceptEdits, AgentChatMode::Agent, false);
    bind_external_process(
        &Process {
            proof: None,
            state: 0,
        },
        &launch,
        false,
        &mut turn,
    )
    .unwrap();
    let p = parameters(&mut turn);
    assert_eq!(p["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(p["approvalPolicy"], "untrusted");
}
#[test]
fn mismatched_run_lock_or_policy_and_dead_or_unknown_process_are_rejected() {
    for variant in 0..8 {
        let (_root, mut launch, request, mut turn) =
            fixture(PermissionMode::AutoAcceptEdits, AgentChatMode::Agent, false);
        let mut p = process(&request);
        match variant {
            0 => launch.run_id = Some("other".into()),
            1 => launch.lock.digest_sha256 = "b".repeat(64),
            2 => launch.permission_binding = None,
            3 => p.state = 1,
            4 => p.state = 2,
            5 => launch.workspace_root = None,
            6 => launch.workspace_access = SandboxWorkspaceAccess::ReadOnly,
            _ => launch.executable = "other-executable".into(),
        }
        assert!(bind_external_process(&p, &launch, true, &mut turn).is_err());
        assert_eq!(
            parameters(&mut turn)["sandboxPolicy"]["type"],
            "workspaceWrite"
        );
    }
}
#[test]
fn external_binding_cannot_be_applied_after_initialization() {
    let (_root, launch, request, mut turn) =
        fixture(PermissionMode::AskEveryTime, AgentChatMode::Agent, false);
    parameters(&mut turn);
    assert!(bind_external_process(&process(&request), &launch, true, &mut turn).is_err());
}

#[test]
fn mandatory_native_evidence_cannot_fall_back_to_sdk_sandbox() {
    let (_root, launch, _, mut turn) =
        fixture(PermissionMode::AskEveryTime, AgentChatMode::Agent, false);
    assert!(
        bind_external_process(
            &Process {
                proof: None,
                state: 0
            },
            &launch,
            true,
            &mut turn
        )
        .is_err()
    );
}
