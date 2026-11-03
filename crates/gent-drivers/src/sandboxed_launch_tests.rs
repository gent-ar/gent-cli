use std::path::PathBuf;
use std::sync::Mutex;

use gent_types::{
    RunVersionLock, SandboxLaunchPolicy, SandboxNetworkPolicy, SandboxResourceLimits,
    SandboxWorkspaceAccess,
};

use super::*;
use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::supervisor::LaunchIntent;

#[derive(Debug)]
struct Process(ProcessSandboxEvidence);
impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}
impl ProviderProcess for Process {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        Some(&self.0)
    }
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}

#[derive(Default)]
struct LaunchPort(Mutex<Vec<(SandboxedLaunchRequest, ProviderLaunch)>>);
impl SandboxedProviderLaunch for LaunchPort {
    type Process = Process;
    fn launch_sandboxed(
        &self,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
    ) -> Result<Process, SandboxedProviderLaunchError> {
        self.0
            .lock()
            .unwrap()
            .push((request.clone(), launch.clone()));
        let mut proof = ProcessSandboxEvidence::new(
            request,
            &request.lock,
            launch.run_id.clone().unwrap(),
            "test-process-creation-1".into(),
            SandboxBackendId::new("test-contained-backend".into()).unwrap(),
        )
        .unwrap();
        if request.profile.gateway().is_some() {
            proof = proof.with_network_peer(
                crate::SandboxNetworkPeer::external_package(
                    "Gent.QA.Proxy.20261029_bh4pzfjvbfcs0".into(),
                    "Gent.QA.Proxy.20261029_1.0.0.0_x64__bh4pzfjvbfcs0".into(),
                    request.profile.gateway().unwrap().peer_nonce().into(),
                    54321,
                    123,
                    456,
                )
                .unwrap(),
            );
        }
        Ok(Process(proof))
    }
}

fn policy() -> SandboxLaunchPolicy {
    SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1,
            max_cpu_time_ms: 1,
        },
    )
    .unwrap()
}

fn request() -> SandboxedLaunchRequest {
    SandboxedLaunchRequest {
        lock: RunVersionLock {
            provider: "codex".into(),
            canonical_path: "/private/codex".into(),
            file_identity: "1:2".into(),
            digest_sha256: "a".repeat(64),
            version: "1".into(),
            compatibility_entry: "codex-1".into(),
        },
        profile: policy()
            .profile_for_workspace(
                &gent_testkit::host_absolute_path("/workspace"),
                SandboxWorkspaceAccess::ReadOnly,
            )
            .unwrap(),
    }
}

fn launch(provider: &str, executable: &str) -> ProviderLaunch {
    ProviderLaunch {
        permission_binding: None,
        run_id: Some("run-1".into()),
        lock: request().lock,
        provider: provider.into(),
        executable: executable.into(),
        arguments: vec![],
        intent: LaunchIntent::Start,
        workspace_root: Some(gent_testkit::host_absolute_path("/workspace")),
        workspace_access: SandboxWorkspaceAccess::ReadOnly,
    }
}

#[test]
fn atomic_port_receives_the_bound_profile_with_the_exact_spawn() {
    let port = LaunchPort::default();
    let launcher = SandboxedLauncher::new(policy(), port);
    launcher.launch(&launch("codex", "/private/codex")).unwrap();
    let calls = launcher.inner.0.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.lock.provider, "codex");
    assert_eq!(calls[0].0.lock, calls[0].1.lock);
    assert_eq!(calls[0].1.executable, PathBuf::from("/private/codex"));
}

#[test]
fn altered_launch_never_reaches_the_atomic_port() {
    let port = LaunchPort::default();
    let launcher = SandboxedLauncher::new(policy(), port);
    assert!(matches!(
        launcher.launch(&launch("claude", "/private/claude")),
        Err(SupervisorError::Lock(LockError::ProviderChanged))
    ));
    assert!(launcher.inner.0.lock().unwrap().is_empty());
}

#[test]
fn each_launch_derives_its_profile_from_its_own_workspace_and_mode() {
    let port = LaunchPort::default();
    let launcher = SandboxedLauncher::new(policy(), port);
    let mut first = launch("codex", "/private/codex");
    first.workspace_root = Some(gent_testkit::host_absolute_path("/workspace-a"));
    let mut second = first.clone();
    second.workspace_root = Some(gent_testkit::host_absolute_path("/workspace-b"));
    second.workspace_access = SandboxWorkspaceAccess::ReadWrite;
    launcher.launch(&first).unwrap();
    launcher.launch(&second).unwrap();
    let calls = launcher.inner.0.lock().unwrap();
    assert_ne!(
        calls[0].0.profile.digest_sha256(),
        calls[1].0.profile.digest_sha256()
    );
}

#[test]
fn missing_run_identity_never_reaches_the_native_port() {
    let launcher = SandboxedLauncher::new(policy(), LaunchPort::default());
    let mut request = launch("codex", "/private/codex");
    request.run_id = None;
    assert!(launcher.launch(&request).is_err());
    assert!(launcher.inner.0.lock().unwrap().is_empty());
}

#[test]
fn full_policy_binding_must_match_before_the_atomic_port() {
    let record = gent_types::PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: gent_types::PolicyScope::ProviderPermissions,
        revision: 1,
        mode: gent_types::PermissionMode::Bypass,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    let binding = gent_types::SandboxPermissionBinding::new(
        "conversation".into(),
        "run-1".into(),
        record.clone(),
        gent_types::HostEpoch(1),
    )
    .unwrap();
    let launcher = SandboxedLauncher::new(
        policy().with_permission_binding(binding.clone()),
        LaunchPort::default(),
    );
    let mut launch = launch("codex", "/private/codex");
    assert!(launcher.launch(&launch).is_err());
    let mut changed = record;
    changed.revision += 1;
    launch.permission_binding = Some(
        gent_types::SandboxPermissionBinding::new(
            "conversation".into(),
            "run-1".into(),
            changed,
            gent_types::HostEpoch(1),
        )
        .unwrap(),
    );
    assert!(launcher.launch(&launch).is_err());
    assert!(launcher.inner.0.lock().unwrap().is_empty());
    launch.permission_binding = Some(binding.clone());
    let process = launcher.launch(&launch).unwrap();
    assert_eq!(
        process.sandbox_evidence().unwrap().permission_binding(),
        Some(&binding)
    );
}

#[test]
fn evidence_cannot_be_rebound_to_another_run_profile_or_lock() {
    let request = request();
    let proof = ProcessSandboxEvidence::new(
        &request,
        &request.lock,
        "run-1".into(),
        "123:456".into(),
        SandboxBackendId::new("test-backend".into()).unwrap(),
    )
    .unwrap();
    assert!(proof.matches(&request, "run-1"));
    assert!(!proof.matches(&request, "run-2"));
    let mut changed = request.clone();
    changed.lock.canonical_path = "/other/codex".into();
    assert!(!proof.matches(&changed, "run-1"));
    changed = request.clone();
    changed.profile = policy()
        .profile_for_workspace(
            &gent_testkit::host_absolute_path("/other"),
            SandboxWorkspaceAccess::ReadOnly,
        )
        .unwrap();
    assert!(!proof.matches(&changed, "run-1"));
}

#[test]
fn an_unproven_returned_process_is_killed_and_never_accepted() {
    #[derive(Debug)]
    struct Unproven(Arc<Mutex<usize>>);
    impl ProcessTreeControl for Unproven {
        fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
            assert_eq!(signal, ProcessTreeSignal::Kill);
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
    }
    impl ProviderProcess for Unproven {
        fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
            Ok(())
        }
        fn close_stdin(&self) -> Result<(), ProcessTreeError> {
            Ok(())
        }
    }
    struct UnprovenPort(Arc<Mutex<usize>>);
    impl SandboxedProviderLaunch for UnprovenPort {
        type Process = Unproven;
        fn launch_sandboxed(
            &self,
            _: &SandboxedLaunchRequest,
            _: &ProviderLaunch,
        ) -> Result<Unproven, SandboxedProviderLaunchError> {
            Ok(Unproven(Arc::clone(&self.0)))
        }
    }
    let killed = Arc::new(Mutex::new(0));
    let launcher = SandboxedLauncher::new(policy(), UnprovenPort(Arc::clone(&killed)));
    assert!(launcher.launch(&launch("codex", "/private/codex")).is_err());
    assert_eq!(*killed.lock().unwrap(), 1);
}

#[test]
fn gateway_binding_cannot_launch_another_run_or_provider() {
    let gateway = gent_types::SandboxProviderGateway::new(
        gent_types::SandboxGatewayApi::Codex,
        "127.0.0.1:54321".parse().unwrap(),
        "c".repeat(64),
        "a".repeat(64),
    )
    .unwrap();
    let policy = SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::ReviewedEgress {
            policy_digest_sha256: "a".repeat(64),
        },
        request().profile.limits(),
    )
    .unwrap();
    let launcher = SandboxedLauncher::new(policy, LaunchPort::default())
        .with_gateway_for_run("run-1".into(), gateway)
        .unwrap();
    let mut other = launch("codex", "/private/codex");
    other.run_id = Some("run-2".into());
    assert!(launcher.launch(&other).is_err());
    other.run_id = Some("run-1".into());
    other.provider = "claude".into();
    other.lock.provider = "claude".into();
    assert!(launcher.launch(&other).is_err());
    assert!(launcher.inner.0.lock().unwrap().is_empty());
    launcher.launch(&launch("codex", "/private/codex")).unwrap();
    assert_eq!(launcher.inner.0.lock().unwrap().len(), 1);
}

#[test]
fn private_state_for_another_run_never_reaches_the_native_port() {
    let root = gent_testkit::host_absolute_path("/private-state");
    let policy = policy()
        .with_resources(vec![
            gent_types::SandboxResourceGrant::PrivateStateReadWrite(root.clone()),
        ])
        .with_private_state(
            gent_types::SandboxPrivateState::new("other-run".into(), root).unwrap(),
        );
    let launcher = SandboxedLauncher::new(policy, LaunchPort::default());
    assert!(launcher.launch(&launch("codex", "/private/codex")).is_err());
    assert!(launcher.inner.0.lock().unwrap().is_empty());
}
