use super::{CleanupContext, PendingHelper};
use gent_types::{
    RunVersionLock, SandboxLaunchPolicy, SandboxNetworkPolicy, SandboxResourceLimits,
    SandboxWorkspaceAccess, SandboxedLaunchRequest,
};
use std::process::{Command, Stdio};

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claurst".into(),
        canonical_path: "C:/qa/does-not-exist/helper.exe".into(),
        file_identity: "1:2".into(),
        digest_sha256: "b".repeat(64),
        version: "1".into(),
        compatibility_entry: "fixture".into(),
    }
}

#[test]
fn a_helper_that_rejects_a_launch_reports_its_typed_reason() {
    let profile = SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1024,
            max_cpu_time_ms: 1000,
        },
    )
    .unwrap()
    .profile_for_workspace(
        &gent_testkit::host_absolute_path("/workspace"),
        SandboxWorkspaceAccess::ReadOnly,
    )
    .unwrap();
    let request = SandboxedLaunchRequest {
        lock: lock(),
        profile,
    };
    let cleanup = CleanupContext::new(&request, lock());
    let child = Command::new("cmd.exe")
        .args([
            "/c",
            "1>&2 echo GENT-SANDBOX-ERROR psec-process-create:5& exit 126",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pin = tempfile::tempfile().unwrap();
    let mut pending = PendingHelper::new(child, &cleanup, pin);
    assert_eq!(
        pending.failure().as_deref(),
        Some("native sandbox rejected launch (psec-process-create:5)")
    );
}
