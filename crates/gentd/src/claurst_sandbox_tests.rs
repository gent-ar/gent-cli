//! Plumbing tests use a fake contained process; they are not native containment acceptance.
use crate::claurst_acp_transport::{ClaurstAcpStdio, ClaurstAcpTransport};
use crate::claurst_permission_policy::{ClaurstPermissionDecision, decide_with_enforcement};
use crate::claurst_sandbox_stdio::ClaurstOwnedAcpStdio;
use crate::sandbox_provider_process::RoutedProviderProcess;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::{ProcessSandboxEvidence, ProviderProcess};
use gent_types::*;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct FakeProcess {
    proof: ProcessSandboxEvidence,
    alive: Arc<AtomicBool>,
    chunks: Mutex<VecDeque<Vec<u8>>>,
}
impl ProcessTreeControl for FakeProcess {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.alive.store(false, Ordering::Release);
        Ok(())
    }
}
impl ProviderProcess for FakeProcess {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.alive.load(Ordering::Acquire).then_some(&self.proof)
    }
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.chunks.lock().unwrap().pop_front())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok((!self.alive.load(Ordering::Acquire)).then_some(Some(0)))
    }
}

fn fixture(
    mode: PermissionMode,
) -> (
    tempfile::TempDir,
    PolicyRecord,
    ClaurstOwnedAcpStdio,
    Arc<AtomicBool>,
) {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let policy = PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    let binding = SandboxPermissionBinding::new(
        "conversation".into(),
        "run".into(),
        policy.clone(),
        HostEpoch(7),
    )
    .unwrap();
    let profile = SandboxLaunchProfile::new(
        &root,
        std::slice::from_ref(&root),
        std::slice::from_ref(&root),
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1024,
            max_cpu_time_ms: 1000,
        },
    )
    .unwrap()
    .with_permission_binding(binding);
    let lock = RunVersionLock {
        provider: "claurst".into(),
        canonical_path: "fake".into(),
        digest_sha256: "a".repeat(64),
        file_identity: "fake-file".into(),
        version: "1".into(),
        compatibility_entry: "test".into(),
    };
    let request = SandboxedLaunchRequest {
        lock: lock.clone(),
        profile,
    };
    let proof = ProcessSandboxEvidence::new(
        &request,
        &lock,
        "run".into(),
        "fake-generation".into(),
        SandboxBackendId::new("fake-test-backend".into()).unwrap(),
    )
    .unwrap();
    let alive = Arc::new(AtomicBool::new(true));
    let process = FakeProcess {
        proof,
        alive: alive.clone(),
        chunks: Mutex::new(VecDeque::from([
            b"{\"id\":".to_vec(),
            b"1}\n{\"id\":2}\n".to_vec(),
        ])),
    };
    (
        directory,
        policy,
        ClaurstOwnedAcpStdio::Contained {
            process: RoutedProviderProcess::new(process),
            pending: vec![],
            fixed_mcp_roster: None,
            model_current: Box::new(|| true),
        },
        alive,
    )
}

#[test]
fn contained_acp_retains_frame_boundaries_and_kills_its_owned_tree() {
    let (_directory, _policy, mut stdio, alive) = fixture(PermissionMode::Autonomous);
    assert_eq!(
        stdio.try_read_frame(128).unwrap(),
        Some(b"{\"id\":1}\n".to_vec())
    );
    assert_eq!(
        stdio.try_read_frame(128).unwrap(),
        Some(b"{\"id\":2}\n".to_vec())
    );
    assert_eq!(stdio.try_read_frame(128).unwrap(), None);
    drop(stdio);
    assert!(!alive.load(Ordering::Acquire));
}

struct SignalRecorder(Arc<Mutex<Vec<ProcessTreeSignal>>>);
impl ProcessTreeControl for SignalRecorder {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.0.lock().unwrap().push(signal);
        Ok(())
    }
}
impl ProviderProcess for SignalRecorder {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        None
    }
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(None)
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok(None)
    }
}

fn recorded(model_current: bool) -> (ClaurstOwnedAcpStdio, Arc<Mutex<Vec<ProcessTreeSignal>>>) {
    let signals = Arc::new(Mutex::new(Vec::new()));
    (
        ClaurstOwnedAcpStdio::Contained {
            process: RoutedProviderProcess::new(SignalRecorder(signals.clone())),
            pending: vec![],
            fixed_mcp_roster: None,
            model_current: Box::new(move || model_current),
        },
        signals,
    )
}

#[test]
fn dropping_contained_acp_requests_the_settling_stop_never_the_unprovable_kill() {
    let (stdio, signals) = recorded(true);
    drop(stdio);
    assert_eq!(*signals.lock().unwrap(), [ProcessTreeSignal::Terminate]);
}

#[test]
fn losing_the_model_settles_the_provider_before_any_further_frame() {
    let (mut stdio, signals) = recorded(false);
    stdio.write_frame(b"{}\n").unwrap();
    assert_eq!(
        signals.lock().unwrap().first(),
        Some(&ProcessTreeSignal::Terminate)
    );
    assert!(!signals.lock().unwrap().contains(&ProcessTreeSignal::Kill));
}

#[test]
fn a_current_model_never_signals_the_provider() {
    let (mut stdio, signals) = recorded(true);
    stdio.write_frame(b"{}\n").unwrap();
    assert_eq!(stdio.exited().unwrap(), None);
    assert!(signals.lock().unwrap().is_empty());
}

#[test]
fn permission_evidence_must_match_live_run_policy_and_host_epoch() {
    let (_directory, policy, stdio, alive) = fixture(PermissionMode::Autonomous);
    let mut transport = ClaurstAcpTransport::new(stdio);
    assert_eq!(
        transport.permission_enforcement("run", &policy, HostEpoch(7)),
        SandboxEnforcement::Enforced
    );
    assert_eq!(
        transport.permission_enforcement("other", &policy, HostEpoch(7)),
        SandboxEnforcement::Unavailable
    );
    assert_eq!(
        transport.permission_enforcement("run", &policy, HostEpoch(8)),
        SandboxEnforcement::Unavailable
    );
    let mut changed = policy.clone();
    changed.revision += 1;
    assert_eq!(
        transport.permission_enforcement("run", &changed, HostEpoch(7)),
        SandboxEnforcement::Unavailable
    );
    alive.store(false, Ordering::Release);
    assert_eq!(
        transport.permission_enforcement("run", &policy, HostEpoch(7)),
        SandboxEnforcement::Unavailable
    );
}

#[test]
fn valid_evidence_uses_the_shared_mode_rules_and_preserves_read_only_posture() {
    for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
        let (directory, policy, stdio, _) = fixture(mode);
        let mut transport = ClaurstAcpTransport::new(stdio);
        let enforcement = transport.permission_enforcement("run", &policy, HostEpoch(7));
        let request =
            PermissionRequest::new("Bash".into(), PermissionCategory::Command, None, None);
        assert_eq!(
            decide_with_enforcement(
                AgentChatMode::Agent,
                &policy,
                &request,
                directory.path(),
                enforcement
            ),
            ClaurstPermissionDecision::Allow
        );
        assert_eq!(
            decide_with_enforcement(
                AgentChatMode::Plan,
                &policy,
                &request,
                directory.path(),
                enforcement
            ),
            ClaurstPermissionDecision::Deny(Some(PermissionDenialReason::ReadOnlyMode))
        );
        assert_eq!(
            decide_with_enforcement(
                AgentChatMode::Agent,
                &policy,
                &request,
                directory.path(),
                SandboxEnforcement::Unavailable
            ),
            ClaurstPermissionDecision::Ask
        );
        let network =
            PermissionRequest::new("Fetch".into(), PermissionCategory::Network, None, None);
        let expected = ClaurstPermissionDecision::Allow;
        assert_eq!(
            decide_with_enforcement(
                AgentChatMode::Agent,
                &policy,
                &network,
                directory.path(),
                enforcement
            ),
            expected
        );
    }
}

#[test]
fn malformed_contained_output_is_bounded_before_json_parsing() {
    let (_directory, _, mut stdio, _) = fixture(PermissionMode::Bypass);
    assert!(stdio.try_read_frame(4).is_err());
}
