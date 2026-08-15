//! Read-only public dependency discovery and explicit-action planning.

use std::process::Command;

use gent_protocol::{
    DependencyActionRequest, DependencyActionResult, DependencyActionState, DependencyPlan,
    DependencyPlanRequest, DependencyProvider,
};
use gent_types::{DependencyStatus, DoctorReport};

#[derive(Clone, Copy, Debug, Default)]
pub struct DependencyCatalog;

impl DependencyCatalog {
    #[allow(clippy::unused_self)]
    #[must_use]
    pub fn doctor(self) -> DoctorReport {
        DoctorReport {
            dependencies: [DependencyProvider::Claude, DependencyProvider::Codex]
                .into_iter()
                .map(discover)
                .chain(std::iter::once(discover_node()))
                .collect(),
        }
    }

    #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
    #[must_use]
    pub fn plan(self, request: DependencyPlanRequest) -> DependencyPlan {
        plan(request.provider, request.action)
    }

    #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
    #[must_use]
    pub fn act(self, request: DependencyActionRequest) -> DependencyActionResult {
        let plan = plan(request.provider, request.action);
        let state = if request.consent_granted {
            DependencyActionState::InstallerNotConfigured
        } else {
            DependencyActionState::ConsentRequired
        };
        DependencyActionResult { plan, state }
    }
}

fn discover(provider: DependencyProvider) -> DependencyStatus {
    let (name, remediation) = match provider {
        DependencyProvider::Claude => (
            "claude",
            "Review the plan, then explicitly run `gent deps install claude --consent`.",
        ),
        DependencyProvider::Codex => (
            "codex",
            "Review the plan, then explicitly run `gent deps install codex --consent`.",
        ),
    };
    dependency_status(name, remediation)
}

fn discover_node() -> DependencyStatus {
    dependency_status(
        "node",
        "Install Node.js explicitly before enabling MCP features.",
    )
}

fn dependency_status(name: &str, remediation: &str) -> DependencyStatus {
    let version = Command::new(name)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned()
        });
    DependencyStatus {
        name: name.into(),
        present: version.is_some(),
        version,
        remediation: remediation.into(),
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

    use super::{DependencyActionState, DependencyCatalog};

    #[test]
    fn plans_are_read_only_and_private_providers_are_unrepresentable() {
        let plan = DependencyCatalog.plan(DependencyPlanRequest {
            provider: DependencyProvider::Claude,
            action: DependencyAction::Install,
        });
        assert!(plan.consent_required);
        assert!(plan.instruction.contains("Anthropic"));
    }

    #[test]
    fn consent_never_silently_starts_an_installer() {
        let result = DependencyCatalog.act(DependencyActionRequest {
            provider: DependencyProvider::Codex,
            action: DependencyAction::Update,
            consent_granted: true,
        });
        assert_eq!(result.state, DependencyActionState::InstallerNotConfigured);
    }
}
