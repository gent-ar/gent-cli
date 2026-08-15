//! Read-only public dependency discovery and explicit-action planning.
use crate::compatibility_assessment::CompatibilityAssessment;
use gent_drivers::lock::capture;
use gent_protocol::{
    DependencyActionRequest, DependencyActionResult, DependencyActionState, DependencyPlan,
    DependencyPlanRequest, DependencyProvider,
};
use gent_types::{
    CompatibilityTrust, DependencyStatus, DoctorNextAction, DoctorReport, ExecutableIdentity,
    McpDoctorStatus, McpPermissionStatus, PrivateBridgeAvailability, PublicProviderStatus,
};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
#[derive(Clone, Debug, Default)]
pub struct DependencyCatalog {
    compatibility: CompatibilityAssessment,
}
impl DependencyCatalog {
    #[must_use]
    pub(crate) fn with_compatibility(compatibility: CompatibilityAssessment) -> Self {
        Self { compatibility }
    }
    #[allow(clippy::unused_self)]
    #[must_use]
    pub fn doctor(&self) -> DoctorReport {
        let providers = [DependencyProvider::Claude, DependencyProvider::Codex]
            .into_iter()
            .map(|provider| observe_provider(provider, &self.compatibility))
            .collect::<Vec<_>>();
        doctor_report(providers, discover_node())
    }
    #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
    #[must_use]
    pub fn plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        plan(request.provider, request.action)
    }
    #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
    #[must_use]
    pub fn act(&self, request: DependencyActionRequest) -> DependencyActionResult {
        let plan = plan(request.provider, request.action);
        let state = if request.consent_granted {
            DependencyActionState::InstallerNotConfigured
        } else {
            DependencyActionState::ConsentRequired
        };
        DependencyActionResult { plan, state }
    }
}
fn doctor_report(
    providers: Vec<(DependencyStatus, PublicProviderStatus)>,
    node: DependencyStatus,
) -> DoctorReport {
    let next_action = providers.iter().find(|(dependency, _)| !dependency.present).map_or_else(
        || DoctorNextAction {
            id: "review-authority-gates".into(),
            instruction: "Review signed compatibility and authority gates before enabling any provider work."
                .into(),
        },
        |(_, provider)| DoctorNextAction {
            id: format!("review-{}-install-plan", provider.provider),
            instruction: format!(
                "Review `gent deps plan install {}`; installation remains explicit and user-controlled.",
                provider.provider
            ),
        },
    );
    DoctorReport {
        dependencies: providers
            .iter()
            .map(|(dependency, _)| dependency.clone())
            .chain(std::iter::once(node))
            .collect(),
        public_providers: providers.into_iter().map(|(_, provider)| provider).collect(),
        mcp: McpDoctorStatus {
            permission: McpPermissionStatus::HardDisabledObserver,
            remediation: "MCP is hard-disabled in observer mode; no connector is inspected, granted permission, or started."
                .into(),
        },
        private_bridge: PrivateBridgeAvailability::NotConfigured,
        next_action,
    }
}
fn observe_provider(
    provider: DependencyProvider,
    compatibility: &CompatibilityAssessment,
) -> (DependencyStatus, PublicProviderStatus) {
    let (name, remediation) = provider_details(provider);
    let executable = find_executable(name);
    let version = executable.as_deref().and_then(probe_version);
    let identity = executable
        .as_deref()
        .and_then(|path| executable_identity(name, path, version.as_deref()));
    let present = executable.is_some();
    let dependency = DependencyStatus {
        name: name.into(),
        present,
        version: version.clone(),
        remediation: remediation.into(),
    };
    let trust = identity
        .as_ref()
        .map_or(CompatibilityTrust::NotConfigured, |identity| {
            compatibility.assess(name, identity)
        });
    let provider = PublicProviderStatus {
        provider: name.into(),
        executable: identity,
        compatibility: trust,
        remediation: if present {
            "A public executable was observed, but no signed compatibility manifest is configured."
                .into()
        } else {
            remediation.into()
        },
    };
    (dependency, provider)
}
fn provider_details(provider: DependencyProvider) -> (&'static str, &'static str) {
    match provider {
        DependencyProvider::Claude => (
            "claude",
            "Review the plan, then explicitly run `gent deps install claude --consent`.",
        ),
        DependencyProvider::Codex => (
            "codex",
            "Review the plan, then explicitly run `gent deps install codex --consent`.",
        ),
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn probe_version(executable: &Path) -> Option<String> {
    Command::new(executable)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8(output.stdout)
                .ok()
                .and_then(|value| value.lines().next().map(str::to_owned))
        })
}

fn executable_identity(
    provider: &str,
    executable: &Path,
    version: Option<&str>,
) -> Option<ExecutableIdentity> {
    let lock = capture(
        provider,
        executable,
        version.unwrap_or("version-unavailable"),
        "unverified",
    )
    .ok()?;
    Some(ExecutableIdentity {
        canonical_path: lock.canonical_path,
        file_identity: lock.file_identity,
        digest_sha256: lock.digest_sha256,
        version: version.map(str::to_owned),
    })
}

fn discover_node() -> DependencyStatus {
    let executable = find_executable("node");
    DependencyStatus {
        name: "node".into(),
        present: executable.is_some(),
        version: executable.as_deref().and_then(probe_version),
        remediation: "Node discovery is read-only; MCP remains hard-disabled until a later authority-gated release."
            .into(),
    }
}

fn plan(provider: DependencyProvider, action: gent_protocol::DependencyAction) -> DependencyPlan {
    let instruction = match (provider, action) {
        (DependencyProvider::Claude, gent_protocol::DependencyAction::Install) => {
            "Use Anthropic's supported Claude Code installer after reviewing its terms."
        }
        (DependencyProvider::Claude, gent_protocol::DependencyAction::Update) => {
            "Use Anthropic's supported Claude Code updater after reviewing its terms."
        }
        (DependencyProvider::Codex, gent_protocol::DependencyAction::Install) => {
            "Use OpenAI's supported Codex installer after reviewing its terms."
        }
        (DependencyProvider::Codex, gent_protocol::DependencyAction::Update) => {
            "Use OpenAI's supported Codex updater after reviewing its terms."
        }
    };
    DependencyPlan {
        provider,
        action,
        instruction: instruction.into(),
        consent_required: true,
    }
}

#[cfg(test)]
mod tests {
    use gent_protocol::{
        DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider,
    };
    use gent_types::{
        CompatibilityTrust, DependencyStatus, ExecutableIdentity, McpPermissionStatus,
        PrivateBridgeAvailability, PublicProviderStatus,
    };

    use super::{DependencyActionState, DependencyCatalog, doctor_report};

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
    fn consent_never_silently_starts_an_installer() {
        let result = DependencyCatalog::default().act(DependencyActionRequest {
            provider: DependencyProvider::Codex,
            action: DependencyAction::Update,
            consent_granted: true,
        });
        assert_eq!(result.state, DependencyActionState::InstallerNotConfigured);
    }
}
