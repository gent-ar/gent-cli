//! Read-only public dependency discovery and explicit-action planning.
use crate::compatibility_assessment::CompatibilityAssessment;
use gent_drivers::lock::capture;
use gent_protocol::{DependencyAction, DependencyPlan, DependencyPlanRequest, DependencyProvider};
use gent_types::{
    CompatibilityTrust, DependencyStatus, DoctorNextAction, DoctorReport, ExecutableIdentity,
    McpDoctorStatus, McpPermissionStatus, PrivateBridgeAvailability, PublicProviderStatus,
};
use std::env;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub struct DependencyCatalog {
    compatibility: CompatibilityAssessment,
}
impl Default for DependencyCatalog {
    fn default() -> Self {
        Self::with_compatibility(CompatibilityAssessment::default())
    }
}

impl DependencyCatalog {
    #[must_use]
    pub(crate) fn with_compatibility(compatibility: CompatibilityAssessment) -> Self {
        Self { compatibility }
    }
}

impl DependencyCatalog {
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
}
pub(crate) fn doctor_report(
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
    // Observer-mode discovery never executes a provider binary, including `--version`.
    // A later, authority-gated lifecycle captures version and rechecks identity before spawn.
    let version = None;
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
        compatibility: trust.clone(),
        remediation: CompatibilityAssessment::remediation(present, &trust, remediation),
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
        version: None,
        remediation: "Node discovery is read-only; MCP remains hard-disabled until a later authority-gated release."
            .into(),
    }
}

fn plan(provider: DependencyProvider, action: DependencyAction) -> DependencyPlan {
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
    DependencyPlan::reviewed(provider, action, instruction, true)
}
