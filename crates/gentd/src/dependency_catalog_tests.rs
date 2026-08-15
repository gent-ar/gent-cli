use gent_drivers::installer::{DependencyInstaller, InstallerError, InstallerInvocation};
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyActionState, DependencyPlanRequest,
    DependencyProvider,
};
use gent_types::{
    CompatibilityTrust, DependencyStatus, ExecutableIdentity, McpPermissionStatus,
    PrivateBridgeAvailability, PublicProviderStatus,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::compatibility_assessment::CompatibilityAssessment;
use crate::dependency_catalog::{DependencyCatalog, doctor_report};

#[derive(Clone, Debug)]
struct TestInstaller {
    result: Result<(), InstallerError>,
    calls: Arc<AtomicUsize>,
}

impl DependencyInstaller for TestInstaller {
    fn execute(&self, _: &InstallerInvocation) -> Result<(), InstallerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}

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
fn consent_controls_the_only_installer_execution_path() {
    let calls = Arc::new(AtomicUsize::new(0));
    let catalog = DependencyCatalog::with_installer(
        CompatibilityAssessment::default(),
        TestInstaller {
            result: Ok(()),
            calls: calls.clone(),
        },
    );
    let mut request = DependencyActionRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Update,
        consent_granted: false,
    };
    assert_eq!(
        catalog.act(&request).state,
        DependencyActionState::ConsentRequired
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    request.consent_granted = true;
    assert_eq!(
        catalog.act(&request).state,
        DependencyActionState::Completed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn installer_failure_is_terminal_and_actionable() {
    let catalog = DependencyCatalog::with_installer(
        CompatibilityAssessment::default(),
        TestInstaller {
            result: Err(InstallerError::Failed("exit status: 1".into())),
            calls: Arc::new(AtomicUsize::new(0)),
        },
    );
    let result = catalog.act(&DependencyActionRequest {
        provider: DependencyProvider::Claude,
        action: DependencyAction::Install,
        consent_granted: true,
    });
    assert_eq!(result.state, DependencyActionState::Failed);
    assert!(result.detail.unwrap().contains("exit status"));
}
