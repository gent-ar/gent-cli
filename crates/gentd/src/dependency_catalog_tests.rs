use crate::dependency_catalog::{DependencyCatalog, doctor_report};
use gent_protocol::{DependencyAction, DependencyPlanRequest, DependencyProvider};
use gent_types::{
    CompatibilityTrust, DependencyStatus, ExecutableIdentity, McpPermissionStatus,
    PrivateBridgeAvailability, PublicProviderStatus,
};

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
