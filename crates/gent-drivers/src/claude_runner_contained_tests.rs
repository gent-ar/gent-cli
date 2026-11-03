//! Runner ownership/approval controls; fake evidence is not native containment proof.
use super::super::*;
use crate::{
    ProcessSandboxEvidence,
    interrupt::{ProcessTreeControl, ProcessTreeError},
};
use gent_types::*;
use std::sync::{Arc, Mutex};
#[derive(Clone, Default)]
struct Seen {
    writes: Arc<Mutex<usize>>,
    kills: Arc<Mutex<usize>>,
    args: Arc<Mutex<Vec<String>>>,
}
struct Process {
    proof: Option<ProcessSandboxEvidence>,
    state: u8,
    seen: Seen,
}
impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        *self.seen.kills.lock().unwrap() += 1;
        Ok(())
    }
}
impl ProviderProcess for Process {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.proof.as_ref()
    }
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        *self.seen.writes.lock().unwrap() += 1;
        Ok(())
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
struct Launcher {
    variant: u8,
    seen: Seen,
}
impl ProcessLauncher for Launcher {
    type Process = Process;
    fn requires_sandbox_evidence(&self, _: &ProviderLaunch) -> bool {
        true
    }
    fn validate_permission_binding(
        &self,
        _: &SandboxPermissionBinding,
    ) -> Result<(), SupervisorError> {
        Ok(())
    }
    fn launch(&self, launch: &ProviderLaunch) -> Result<Process, SupervisorError> {
        *self.seen.args.lock().unwrap() = launch.arguments.clone();
        let mut policy = SandboxLaunchPolicy::new(
            vec![],
            SandboxNetworkPolicy::Disabled,
            SandboxResourceLimits {
                max_processes: 1,
                max_memory_bytes: 1,
                max_cpu_time_ms: 1,
            },
        )
        .unwrap()
        .with_permission_binding(launch.permission_binding.clone().unwrap());
        if self.variant == 4 {
            let mut binding = launch.permission_binding.as_ref().unwrap().clone();
            let mut record = binding.policy().clone();
            record.policy_id = "other".into();
            binding = SandboxPermissionBinding::new(
                "conversation".into(),
                "run".into(),
                record,
                HostEpoch(1),
            )
            .unwrap();
            policy = policy.with_permission_binding(binding);
        }
        let mut request = SandboxedLaunchRequest {
            lock: launch.lock.clone(),
            profile: policy
                .profile_for_workspace(
                    launch.workspace_root.as_ref().unwrap(),
                    launch.workspace_access,
                )
                .unwrap(),
        };
        if self.variant == 3 {
            request.lock.digest_sha256 = "b".repeat(64);
        }
        let proof = if self.variant == 1 {
            None
        } else {
            Some(
                ProcessSandboxEvidence::new(
                    &request,
                    &request.lock,
                    "run".into(),
                    "pid-birth".into(),
                    SandboxBackendId::new("fixture".into()).unwrap(),
                )
                .unwrap(),
            )
        };
        Ok(Process {
            proof,
            state: if self.variant == 2 {
                1
            } else if self.variant == 5 {
                2
            } else {
                0
            },
            seen: self.seen.clone(),
        })
    }
}
fn run(
    mode: PermissionMode,
    role: AgentChatMode,
    resume: bool,
    variant: u8,
) -> (Result<(), ClaudeRunnerError>, Seen) {
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("claude.exe");
    std::fs::write(&image, b"immutable fixture").unwrap();
    let lock = crate::lock::capture("claude", &image, "1", "entry").unwrap();
    let workspace = root.path().canonicalize().unwrap();
    let seen = Seen::default();
    let mut runner = ClaudeStreamRunner::new(
        Launcher {
            variant,
            seen: seen.clone(),
        },
        BufferPolicy::new(
            64,
            MAX_PROVIDER_FRAME_BYTES,
            32,
            MAX_PROVIDER_FRAME_BYTES / 2,
        )
        .unwrap(),
    );
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
    runner.prepare_permission_binding(binding).unwrap();
    let result = runner.start(ClaudeRunStart {
        run_id: "run".into(),
        lock,
        prompt: "fixture".into(),
        content: vec![],
        turn_options:
            crate::claude_turn_options::ClaudeTurnOptions::from_selection_with_permissions(
                &AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "explicit-fixture".into(),
                    effort: AgentChatEffort::Low,
                    mode: role,
                },
                mode,
            )
            .unwrap(),
        goal: None,
        fresh_context: None,
        intent: if resume {
            LaunchIntent::Resume {
                session_id: "session".into(),
            }
        } else {
            LaunchIntent::Start
        },
        workspace_root: workspace,
        workspace_access: SandboxWorkspaceAccess::from_mode(role),
        mcp_config: None,
    });
    (result, seen)
}
#[test]
fn contained_claude_start_and_resume_preserve_all_four_permission_modes() {
    for role in [
        AgentChatMode::Ask,
        AgentChatMode::Plan,
        AgentChatMode::Agent,
    ] {
        for (mode, approval) in [
            (PermissionMode::AskEveryTime, "manual"),
            (PermissionMode::AutoAcceptEdits, "manual"),
            (PermissionMode::Autonomous, "manual"),
            (PermissionMode::Bypass, "bypassPermissions"),
        ] {
            for resume in [false, true] {
                let (result, seen) = run(mode, role, resume, 0);
                result.unwrap();
                assert_eq!(*seen.writes.lock().unwrap(), 1);
                let args = seen.args.lock().unwrap();
                let index = args.iter().position(|s| s == "--permission-mode").unwrap();
                assert_eq!(
                    args[index + 1],
                    if role == AgentChatMode::Plan {
                        "plan"
                    } else {
                        approval
                    }
                );
                assert!(
                    args.windows(2)
                        .any(|p| p == ["--permission-prompt-tool", "stdio"])
                );
            }
        }
    }
}
#[test]
fn contained_claude_missing_mismatched_dead_or_unknown_proof_never_sends_prompt() {
    for variant in 1..=5 {
        let (result, seen) = run(
            PermissionMode::AskEveryTime,
            AgentChatMode::Agent,
            false,
            variant,
        );
        assert!(result.is_err());
        assert_eq!(*seen.writes.lock().unwrap(), 0);
        assert_eq!(*seen.kills.lock().unwrap(), 1);
    }
}
