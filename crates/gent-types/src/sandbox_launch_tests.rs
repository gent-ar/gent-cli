use std::path::{Path, PathBuf};

use crate::{
    RunVersionLock, SandboxBackendId, SandboxEnforcement, SandboxLaunchContractError,
    SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};

pub(crate) fn host_absolute_path(posix: &str) -> PathBuf {
    if cfg!(windows) {
        Path::new("C:\\").join(posix.trim_start_matches('/'))
    } else {
        PathBuf::from(posix)
    }
}

fn profile() -> SandboxLaunchProfile {
    SandboxLaunchProfile::new(
        &host_absolute_path("/workspace"),
        &[host_absolute_path("/workspace")],
        &[host_absolute_path("/workspace/project")],
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
        &host_absolute_path("/workspace"),
        &[host_absolute_path("/workspace")],
        &[host_absolute_path("/workspace/project")],
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
            &host_absolute_path("/workspace"),
            &[host_absolute_path("/workspace")],
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
            &host_absolute_path("/workspace"),
            &[
                host_absolute_path("/workspace"),
                host_absolute_path("/other")
            ],
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
            &host_absolute_path("/workspace"),
            &[host_absolute_path("/workspace")],
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

#[test]
fn explicit_resource_grants_bind_access_and_reject_overlapping_authority() {
    use crate::SandboxResourceGrant::{
        ConfigurationReadOnly, PrivateStateReadWrite, RuntimeReadOnly,
    };
    let runtime = RuntimeReadOnly(host_absolute_path("/provider-runtime"));
    let private = PrivateStateReadWrite(host_absolute_path("/provider-private"));
    let first = profile()
        .with_resources(vec![runtime.clone(), private.clone()])
        .unwrap();
    let reordered = profile()
        .with_resources(vec![private.clone(), runtime.clone(), runtime])
        .unwrap();
    assert_eq!(first.digest_sha256(), reordered.digest_sha256());
    assert_ne!(first.digest_sha256(), profile().digest_sha256());
    assert_eq!(first.resources().len(), 2);
    assert!(
        profile()
            .with_resources(vec![
                ConfigurationReadOnly(host_absolute_path("/provider-private/config")),
                private,
            ])
            .is_err()
    );
    assert!(
        profile()
            .with_resources(vec![PrivateStateReadWrite(host_absolute_path(
                "/workspace/private"
            )),])
            .is_err()
    );
    assert!(
        profile()
            .with_resources(vec![RuntimeReadOnly(host_absolute_path("/")),])
            .is_err()
    );
}

#[test]
fn sandbox_gateway_requires_reviewed_binding_and_redacts_the_scoped_token() {
    use crate::{
        SandboxGatewayApi, SandboxLaunchPolicy, SandboxProviderGateway, SandboxWorkspaceAccess,
    };
    let gateway = SandboxProviderGateway::new(
        SandboxGatewayApi::Claude,
        "127.0.0.1:54321".parse().unwrap(),
        "c".repeat(64),
        "a".repeat(64),
    )
    .unwrap();
    assert!(!format!("{gateway:?}").contains(&"c".repeat(64)));
    assert!(profile().with_gateway(gateway.clone()).is_err());
    let policy = SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::ReviewedEgress {
            policy_digest_sha256: "a".repeat(64),
        },
        profile().limits(),
    )
    .unwrap();
    let first = policy
        .profile_for_workspace(
            &host_absolute_path("/workspace"),
            SandboxWorkspaceAccess::ReadOnly,
        )
        .unwrap();
    let second = first.clone().with_gateway(gateway).unwrap();
    assert_ne!(first.digest_sha256(), second.digest_sha256());
    assert!(
        SandboxProviderGateway::new(
            SandboxGatewayApi::Claude,
            "192.0.2.1:54321".parse().unwrap(),
            "c".repeat(64),
            "a".repeat(64)
        )
        .is_err()
    );
}

#[test]
fn sandbox_private_state_is_run_bound_and_cannot_outlive_its_exact_resource_grant() {
    use crate::{SandboxPrivateState, SandboxResourceGrant};
    let root = host_absolute_path("/private-state");
    let state = SandboxPrivateState::new("run-1".into(), root.clone()).unwrap();
    assert_eq!(state.directories().len(), 6);
    assert!(profile().with_private_state(state.clone()).is_err());
    let granted = profile()
        .with_resources(vec![SandboxResourceGrant::PrivateStateReadWrite(
            root.clone(),
        )])
        .unwrap();
    let bound = granted.clone().with_private_state(state).unwrap();
    assert_ne!(granted.digest_sha256(), bound.digest_sha256());
    assert_eq!(bound.private_state().unwrap().run_id(), "run-1");
    assert!(bound.with_resources(vec![]).is_err());
    assert!(SandboxPrivateState::new("../run".into(), root).is_err());
}

#[test]
#[cfg(target_pointer_width = "64")]
fn strict_cpu_default_preserves_existing_profile_digest() {
    let value = profile();
    assert_eq!(
        value.cpu_semantics(),
        crate::SandboxCpuSemantics::StrictTotal
    );
    let expected = if cfg!(windows) {
        "e3dd2ef9a402bedeef4a2b634a31ed0f3105bae1a195bfae981a0d847f9dff6f"
    } else {
        "fa4fd179f55b54cffeda9a243669702b81708491de4fcb691655a653c079bb68"
    };
    assert_eq!(value.digest_sha256(), expected);
    assert_eq!(
        value
            .clone()
            .with_cpu_semantics(crate::SandboxCpuSemantics::StrictTotal)
            .unwrap()
            .digest_sha256(),
        expected
    );
}
#[test]
fn monitored_cpu_sampling_and_wall_are_bound_into_profile_identity() {
    let value = profile();
    let monitored = value
        .clone()
        .with_cpu_semantics(crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 5,
            wall_ms: 60000,
        })
        .unwrap();
    assert_ne!(value.digest_sha256(), monitored.digest_sha256());
    for semantics in [
        crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 6,
            wall_ms: 60000,
        },
        crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 5,
            wall_ms: 60001,
        },
    ] {
        assert_ne!(
            monitored.digest_sha256(),
            value
                .clone()
                .with_cpu_semantics(semantics)
                .unwrap()
                .digest_sha256()
        );
    }
    for semantics in [
        crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 0,
            wall_ms: 60000,
        },
        crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 101,
            wall_ms: 60000,
        },
        crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 5,
            wall_ms: 99,
        },
        crate::SandboxCpuSemantics::MonitoredTotal {
            poll_ms: 5,
            wall_ms: 600_001,
        },
    ] {
        assert!(value.clone().with_cpu_semantics(semantics).is_err());
    }
}
