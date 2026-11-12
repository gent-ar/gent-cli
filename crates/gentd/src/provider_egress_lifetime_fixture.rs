use super::super::*;
use gent_drivers::ProviderProcess;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::sandboxed_launch::{ProcessSandboxEvidence, SandboxedProviderLaunchError};
use gent_drivers::supervisor::{LaunchIntent, ProviderLaunch};
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_types::*;
use std::sync::{Mutex, Weak};

pub(super) struct Fixture {
    pub(super) lease: Arc<ProviderGatewayLease>,
    pub(super) launch: ProviderLaunch,
    pub(super) ancestor: tempfile::TempDir,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let ancestor = tempfile::tempdir().unwrap();
        let workspace = ancestor.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let executable = workspace.join("provider.exe");
        std::fs::write(&executable, b"owned nonexecuted fixture").unwrap();
        let lock = gent_drivers::lock::capture("claude", &executable, "1", "fixture").unwrap();
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
                        provider: AgentChatProvider::Claude,
                        model: "fixture".into(),
                        effort: AgentChatEffort::Low,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace".into(),
                    canonical_path: workspace.to_str().unwrap().into(),
                },
            )
            .unwrap();
        let policy = PolicyRecord {
            policy_id: "policy".into(),
            workspace_id: "workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::Bypass,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        };
        ledger.save_policy(&policy).unwrap();
        let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
        let binding = SandboxPermissionBinding::new(
            "conversation".into(),
            "run".into(),
            policy,
            HostEpoch(1),
        )
        .unwrap();
        let fence = RunFence::capture(ledger, coordinator, &binding, lock.clone()).unwrap();
        let private_root = tempfile::Builder::new()
            .prefix("private-")
            .tempdir_in(ancestor.path())
            .unwrap();
        let private_state =
            SandboxPrivateState::new("run".into(), private_root.path().canonicalize().unwrap())
                .unwrap();
        for directory in private_state.directories() {
            std::fs::create_dir(directory).unwrap();
        }
        let lease = Arc::new(ProviderGatewayLease {
            fence: Arc::new(fence),
            broker: ProviderEgressBroker::start(super::super::super::tests::policy(
                ProviderApi::Claude,
                "127.0.0.1:9".parse().unwrap(),
            ))
            .unwrap(),
            revoked: AtomicBool::new(false),
            private_state,
            _private_root: private_root,
            retained_resources: Mutex::new(Vec::new()),
            contained_model: None,
            #[cfg(any(windows, target_os = "linux"))]
            mcp: std::sync::Mutex::default(),
            #[cfg(target_os = "linux")]
            mcp_deployment: Default::default(),
        });
        let launch = ProviderLaunch {
            permission_binding: Some(binding),
            run_id: Some("run".into()),
            provider: "claude".into(),
            executable: lock.canonical_path.clone().into(),
            lock,
            arguments: vec![],
            intent: LaunchIntent::Start,
            workspace_root: Some(workspace),
            workspace_access: SandboxWorkspaceAccess::ReadWrite,
        };
        Self {
            lease,
            launch,
            ancestor,
        }
    }
}

pub(super) struct Process {
    pub(super) stop_ok: bool,
    pub(super) exit: Result<Option<i32>, ()>,
    pub(super) proof: Option<ProcessSandboxEvidence>,
}

impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        if self.stop_ok {
            Ok(())
        } else {
            Err(ProcessTreeError::Failed("unproven stop".into()))
        }
    }
}

impl ProviderProcess for Process {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.exit
            .map(|code| code.map(Some))
            .map_err(|()| ProcessTreeError::Failed("unproven exit".into()))
    }
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.proof.as_ref()
    }
}

pub(super) struct Backend {
    pub(super) revoke: Weak<ProviderGatewayLease>,
    pub(super) fail: bool,
}

impl SandboxedProviderLaunch for Backend {
    type Process = Process;
    fn launch_sandboxed(
        &self,
        request: &SandboxedLaunchRequest,
        _: &ProviderLaunch,
    ) -> Result<Process, SandboxedProviderLaunchError> {
        if self.fail {
            return Err(SandboxedProviderLaunchError::ContainmentRejected);
        }
        let proof = ProcessSandboxEvidence::new(
            request,
            &request.lock,
            "run".into(),
            "fixture".into(),
            SandboxBackendId::new("fixture".into()).unwrap(),
        )
        .unwrap()
        .with_network_peer(
            gent_drivers::SandboxNetworkPeer::external_package(
                "Gent.Test_bh4pzfjvbfcs0".into(),
                "Gent.Test_1.0.0.0_x64__bh4pzfjvbfcs0".into(),
                request.profile.gateway().unwrap().peer_nonce().into(),
                65000,
                1,
                1,
            )
            .unwrap(),
        );
        self.revoke.upgrade().unwrap().revoke();
        Ok(Process {
            stop_ok: false,
            exit: Ok(None),
            proof: Some(proof),
        })
    }
}

pub(super) fn limits() -> SandboxResourceLimits {
    SandboxResourceLimits {
        max_processes: 4,
        max_memory_bytes: 1024 * 1024,
        max_cpu_time_ms: 1000,
    }
}
