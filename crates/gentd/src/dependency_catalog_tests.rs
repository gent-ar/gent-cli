use crate::dependency_catalog::{DependencyCatalog, doctor_report};
use gent_protocol::{DependencyAction, DependencyPlanRequest, DependencyProvider};
use gent_types::{
    CompatibilityTrust, DependencyStatus, ExecutableIdentity, McpPermissionStatus,
    PrivateBridgeAvailability, PublicProviderStatus,
};
use std::fs;

fn provider(present: bool) -> (DependencyStatus, PublicProviderStatus) {
    (
        DependencyStatus {
            name: "claude".into(),
            present,
            version: present.then(|| "1.2.3".into()),
            remediation: "review plan".into(),
        },
        PublicProviderStatus {
            provider: "claude".into(),
            executable: present.then(|| ExecutableIdentity {
                canonical_path: "/public/claude".into(),
                file_identity: "10:20".into(),
                digest_sha256: "abc".into(),
                version: Some("1.2.3".into()),
            }),
            compatibility: CompatibilityTrust::NotConfigured,
            remediation: "review manifest".into(),
        },
    )
}

fn node() -> DependencyStatus {
    DependencyStatus {
        name: "node".into(),
        present: true,
        version: Some("v22".into()),
        remediation: "none".into(),
    }
}

#[test]
fn doctor_reports_provenance_gates_and_a_safe_next_action() {
    let report = doctor_report(vec![provider(false)], node());
    assert_eq!(
        report.public_providers[0].compatibility,
        CompatibilityTrust::NotConfigured
    );
    assert!(report.public_providers[0].executable.is_none());
    assert_eq!(
        report.mcp.permission,
        McpPermissionStatus::HardDisabledObserver
    );
    assert_eq!(
        report.private_bridge,
        PrivateBridgeAvailability::NotConfigured
    );
    assert_eq!(report.next_action.id, "review-claude-install-plan");
}

#[test]
fn installed_public_provider_preserves_identity_without_claiming_trust() {
    let report = doctor_report(vec![provider(true)], node());
    let identity = report.public_providers[0].executable.as_ref().unwrap();
    assert_eq!(identity.digest_sha256, "abc");
    assert_eq!(
        report.public_providers[0].compatibility,
        CompatibilityTrust::NotConfigured
    );
    assert_eq!(report.next_action.id, "review-authority-gates");
}

#[test]
fn plans_are_read_only_and_private_providers_are_unrepresentable() {
    let plan = DependencyCatalog::default().plan(DependencyPlanRequest {
        provider: DependencyProvider::Claude,
        action: DependencyAction::Install,
    });
    assert!(plan.consent_required);
    assert!(plan.instruction.contains("Anthropic"));
}

#[test]
fn doctor_prefers_the_private_gent_provider_prefix() {
    let root = tempfile::tempdir().unwrap();
    let prefix = root.path().join("npm-global");
    let claude = crate::standalone_provider_setup::provider_executable(
        &prefix,
        gent_protocol::DependencyProvider::Claude,
    )
    .unwrap();
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(claude, "provider").unwrap();
    let report =
        DependencyCatalog::with_private_prefix(crate::CompatibilityAssessment::default(), prefix)
            .doctor();
    assert!(
        report
            .public_providers
            .iter()
            .any(|provider| provider.provider == "claude" && provider.executable.is_some())
    );
}

#[test]
fn private_gent_provider_prefix_never_falls_back_to_a_host_cli() {
    let root = tempfile::tempdir().unwrap();
    let report = DependencyCatalog::with_private_prefix(
        crate::CompatibilityAssessment::default(),
        root.path().join("npm-global"),
    )
    .doctor();
    assert!(
        report
            .public_providers
            .iter()
            .all(|provider| provider.executable.is_none())
    );
}

#[test]
fn standalone_doctor_reports_the_composed_daemon() {
    let root = tempfile::tempdir().unwrap();
    let claude = root.path().join("claude");
    fs::write(&claude, "provider").unwrap();
    let node = root.path().join("runtime/node/bin/node");
    fs::create_dir_all(node.parent().unwrap()).unwrap();
    fs::write(&node, "node").unwrap();
    let report = DependencyCatalog::with_private_prefix(
        crate::CompatibilityAssessment::default(),
        root.path().join("npm-global"),
    )
    .for_standalone(crate::dependency_catalog::standalone::StandaloneDoctor {
        executables: crate::provider_executables::ProviderExecutables::explicit(Some(claude), None),
        node: Some(node),
        mcp_servers: vec!["gent-goal".into(), "gent-forge".into()],
        gent_runtime: true,
        provider_release: true,
        provisioned: None,
    })
    .doctor();
    let claude = &report.public_providers[0];
    assert_eq!(claude.provider, "claude");
    assert!(claude.executable.is_some());
    let node = report
        .dependencies
        .iter()
        .find(|d| d.name == "node")
        .unwrap();
    assert!(node.present);
    assert_eq!(report.mcp.permission, McpPermissionStatus::Enabled);
    assert!(report.mcp.remediation.contains("gent-goal"));
    assert_eq!(report.private_bridge, PrivateBridgeAvailability::Available);
    assert!(
        report
            .dependencies
            .iter()
            .all(|d| !d.remediation.contains("deps install"))
    );
    let codex = report
        .dependencies
        .iter()
        .find(|d| d.name == "codex")
        .unwrap();
    assert!(!codex.present);
    assert!(codex.remediation.contains("send a prompt"));
    assert!(report.next_action.instruction.contains("send a prompt"));
    assert_eq!(
        gent_types::OnboardingState::from_doctor(&report).branches[0].readiness,
        gent_types::OnboardingReadiness::PrivateBridgeAvailable
    );
}

struct Provisioned(std::path::PathBuf);

impl gent_ports::ProvisionedProviderLockReader for Provisioned {
    fn find_provisioned_provider_installation(
        &self,
        provider: &str,
    ) -> Result<Option<gent_types::ProvisionedProviderInstallation>, gent_ports::LedgerError> {
        Ok(Some(gent_types::ProvisionedProviderInstallation {
            lock: gent_types::ProvisionedProviderLock {
                run_lock: gent_types::RunVersionLock {
                    provider: provider.into(),
                    canonical_path: self
                        .0
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    file_identity: "provisioned".into(),
                    digest_sha256: "d".repeat(64),
                    version: "0.154.0".into(),
                    compatibility_entry: "codex-0.154.0".into(),
                },
            },
            provenance: gent_types::ProviderInstallProvenance {
                package_name: "@openai/codex".into(),
                package_version: "0.154.0".into(),
                package_integrity: "sha512-test".into(),
                package_policy_digest_sha256: "a".repeat(64),
                node_runtime_digest_sha256: "b".repeat(64),
                release_artifact_digest_sha256: "c".repeat(64),
                receipt_fingerprint_sha256: "e".repeat(64),
            },
        }))
    }
}

#[test]
fn standalone_doctor_identifies_a_provisioned_provider_by_its_recorded_version() {
    let root = tempfile::tempdir().unwrap();
    let codex = crate::standalone_provider_setup::provider_executable(
        &root.path().join("npm-global"),
        gent_protocol::DependencyProvider::Codex,
    )
    .unwrap();
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::write(&codex, "provider").unwrap();
    let report = DependencyCatalog::with_private_prefix(
        crate::CompatibilityAssessment::default(),
        root.path().join("npm-global"),
    )
    .for_standalone(crate::dependency_catalog::standalone::StandaloneDoctor {
        provider_release: true,
        provisioned: Some(std::sync::Arc::new(Provisioned(codex))),
        ..Default::default()
    })
    .doctor();
    let codex = report
        .public_providers
        .iter()
        .find(|provider| provider.provider == "codex")
        .unwrap();
    assert_eq!(
        codex.executable.as_ref().unwrap().version.as_deref(),
        Some("0.154.0")
    );
}
