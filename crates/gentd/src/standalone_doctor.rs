use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gent_ports::ProvisionedProviderLockReader;

use gent_protocol::DependencyProvider;
use gent_types::{
    DependencyStatus, DoctorNextAction, DoctorReport, McpDoctorStatus, McpPermissionStatus,
    PrivateBridgeAvailability, PublicProviderStatus,
};

#[derive(Clone, Default)]
pub(crate) struct StandaloneDoctor {
    pub(crate) executables: crate::provider_executables::ProviderExecutables,
    pub(crate) node: Option<PathBuf>,
    pub(crate) mcp_servers: Vec<String>,
    pub(crate) gent_runtime: bool,
    pub(crate) provider_release: bool,
    pub(crate) provisioned: Option<Arc<dyn ProvisionedProviderLockReader>>,
}

impl std::fmt::Debug for StandaloneDoctor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StandaloneDoctor(..)")
    }
}

impl StandaloneDoctor {
    pub(crate) fn provisioned_version(&self, provider: &str, executable: &Path) -> Option<String> {
        let lock = self
            .provisioned
            .as_ref()?
            .find_provisioned_provider_installation(provider)
            .ok()??
            .lock
            .run_lock;
        (Path::new(&lock.canonical_path) == executable.canonicalize().ok()?).then_some(lock.version)
    }

    pub(crate) fn executable(&self, provider: DependencyProvider) -> Option<&Path> {
        self.executables.explicit_path(match provider {
            DependencyProvider::Claude => gent_types::AgentChatProvider::Claude,
            DependencyProvider::Codex => gent_types::AgentChatProvider::Codex,
        })
    }

    pub(crate) fn remediation(&self, provider: &str, present: bool) -> String {
        if present {
            format!("Gentd runs this {provider} executable for {provider} conversations.")
        } else if self.provider_release {
            format!(
                "Choose {provider} for a conversation and send a prompt; Gentd installs its signed release before the first turn."
            )
        } else {
            format!(
                "This Gent build has no signed provider release; start gentd with --standalone-{provider}-executable to use an installed {provider}."
            )
        }
    }

    pub(crate) fn report(
        &self,
        providers: Vec<(DependencyStatus, PublicProviderStatus)>,
        node: DependencyStatus,
    ) -> DoctorReport {
        let next_action = providers
            .iter()
            .find(|(dependency, _)| !dependency.present)
            .map_or_else(
                || DoctorNextAction {
                    id: "start-conversation".into(),
                    instruction: "Run `gent` to start a conversation.".into(),
                },
                |(dependency, _)| DoctorNextAction {
                    id: format!("provision-{}", dependency.name),
                    instruction: dependency.remediation.clone(),
                },
            );
        DoctorReport {
            dependencies: providers
                .iter()
                .map(|(dependency, _)| dependency.clone())
                .chain(std::iter::once(node))
                .collect(),
            public_providers: providers
                .into_iter()
                .map(|(_, provider)| provider)
                .collect(),
            mcp: McpDoctorStatus {
                permission: McpPermissionStatus::Enabled,
                remediation: if self.mcp_servers.is_empty() {
                    "No MCP servers are configured for provider sessions.".into()
                } else {
                    format!(
                        "Gentd starts these MCP servers for provider sessions: {}.",
                        self.mcp_servers.join(", ")
                    )
                },
            },
            private_bridge: if self.gent_runtime {
                PrivateBridgeAvailability::Available
            } else {
                PrivateBridgeAvailability::NotConfigured
            },
            next_action,
        }
    }
}
