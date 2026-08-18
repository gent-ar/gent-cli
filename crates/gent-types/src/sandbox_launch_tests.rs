use std::path::PathBuf;

use crate::{
    RunVersionLock, SandboxBackendId, SandboxEnforcement, SandboxLaunchContractError,
    SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};

fn profile() -> SandboxLaunchProfile {
    SandboxLaunchProfile::new(
        &PathBuf::from("/workspace"),
        &[PathBuf::from("/workspace")],
        &[PathBuf::from("/workspace/project")],
        vec!["TERM".into(), "LANG".into()],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 8,
            max_memory_bytes: 1_000_000,
            max_cpu_time_ms: 60_000,
        },
    )
    .unwrap()
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "codex".into(),
        canonical_path: "/private/bin/codex".into(),
        file_identity: "10:1".into(),
        digest_sha256: "a".repeat(64),
        version: "1".into(),
        compatibility_entry: "entry".into(),
    }
}

#[test]
fn profile_is_deterministic_and_rejects_ambient_credentials() {
    let first = profile();
    let reordered = SandboxLaunchProfile::new(
        &PathBuf::from("/workspace"),
        &[PathBuf::from("/workspace")],
        &[PathBuf::from("/workspace/project")],
        vec!["LANG".into(), "TERM".into(), "TERM".into()],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 8,
            max_memory_bytes: 1_000_000,
            max_cpu_time_ms: 60_000,
        },
    )
    .unwrap();
    assert_eq!(first.digest_sha256(), reordered.digest_sha256());
    assert!(matches!(
        SandboxLaunchProfile::new(
            &PathBuf::from("/workspace"),
            &[PathBuf::from("/workspace")],
            &[],
            vec!["AWS_SECRET_ACCESS_KEY".into()],
            SandboxNetworkPolicy::Disabled,
            SandboxResourceLimits {
                max_processes: 1,
                max_memory_bytes: 1,
                max_cpu_time_ms: 1,
            },
        ),
        Err(SandboxLaunchContractError::InvalidEnvironment)
    ));
}

#[test]
fn preflight_attestation_requires_enforcement_and_an_exact_lock_recheck() {
    let request = SandboxedLaunchRequest {
        lock: lock(),
        profile: profile(),
    };
    let backend = SandboxBackendId::new("macos-signed-helper-v1".into()).unwrap();
    assert!(matches!(
        request.attest_after_lock_recheck(
            &lock(),
            backend.clone(),
            SandboxEnforcement::Unavailable
        ),
        Err(SandboxLaunchContractError::NotEnforced)
    ));
    let mut changed = lock();
    changed.file_identity = "11:1".into();
    assert!(matches!(
        request.attest_after_lock_recheck(&changed, backend.clone(), SandboxEnforcement::Enforced),
        Err(SandboxLaunchContractError::LockChanged)
    ));
    let attestation = request
        .attest_after_lock_recheck(&lock(), backend, SandboxEnforcement::Enforced)
        .unwrap();
    assert_eq!(attestation.executable_digest_sha256, "a".repeat(64));
    assert_eq!(
        attestation.profile_digest_sha256,
        request.profile.digest_sha256()
    );
}

#[test]
fn profile_rejects_outside_workspace_and_unreviewed_egress() {
    assert!(matches!(
        SandboxLaunchProfile::new(
            &PathBuf::from("/workspace"),
            &[PathBuf::from("/workspace"), PathBuf::from("/other")],
            &[],
            vec![],
            SandboxNetworkPolicy::Disabled,
            SandboxResourceLimits {
                max_processes: 1,
                max_memory_bytes: 1,
                max_cpu_time_ms: 1,
            },
        ),
        Err(SandboxLaunchContractError::InvalidRoots)
    ));
    assert!(matches!(
        SandboxLaunchProfile::new(
            &PathBuf::from("/workspace"),
            &[PathBuf::from("/workspace")],
            &[],
            vec![],
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256: "not-a-digest".into(),
            },
            SandboxResourceLimits {
                max_processes: 1,
                max_memory_bytes: 1,
                max_cpu_time_ms: 1,
            },
        ),
        Err(SandboxLaunchContractError::InvalidEgressPolicy)
    ));
}
