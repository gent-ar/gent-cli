//! Provider-specific installation commands with no shell interpolation.

use std::time::{SystemTime, UNIX_EPOCH};

use gent_drivers::installer::{DependencyInstaller, NpmGlobalPrefix};
use gent_ports::{
    DependencyActionExecutor, DependencyActionExecutorError, DependencyActionOperation,
    PackageInstallPolicy,
};

/// Shell-free daemon adapter from a signed package policy to the public installer driver.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct SystemDependencyExecutor<I, P> {
    installer: I,
    npm: Option<NpmGlobalPrefix>,
    policy: P,
}

#[allow(dead_code)]
impl<I, P> SystemDependencyExecutor<I, P> {
    #[must_use]
    pub(crate) fn new(installer: I, npm: Option<NpmGlobalPrefix>, policy: P) -> Self {
        Self {
            installer,
            npm,
            policy,
        }
    }
}

impl<I: DependencyInstaller, P: PackageInstallPolicy> DependencyActionExecutor
    for SystemDependencyExecutor<I, P>
{
    fn execute(
        &self,
        operation: &DependencyActionOperation,
    ) -> Result<(), DependencyActionExecutorError> {
        operation
            .action
            .parse::<gent_protocol::DependencyAction>()
            .map_err(
                |error: gent_protocol::ProtocolError| DependencyActionExecutorError {
                    message: error.to_string(),
                },
            )?;
        let package = self
            .policy
            .approved_package(&operation.provider, unix_seconds())
            .map_err(|error| DependencyActionExecutorError {
                message: error.to_string(),
            })?;
        if package.provider != operation.provider {
            return Err(DependencyActionExecutorError {
                message: "signed package policy selected a different provider".into(),
            });
        }
        let npm = self
            .npm
            .as_ref()
            .ok_or_else(|| DependencyActionExecutorError {
                message: "bundled Node runtime is unavailable; set GENT_NODE_BINARY".into(),
            })?;
        self.installer
            .install(npm, &package)
            .map_err(|error| DependencyActionExecutorError {
                message: error.to_string(),
            })
    }
}

/// Denies dependency effects in the shipped observer daemon.
#[derive(Clone, Debug, Default)]
pub(crate) struct ObserverDependencyExecutor;

impl DependencyActionExecutor for ObserverDependencyExecutor {
    fn execute(&self, _: &DependencyActionOperation) -> Result<(), DependencyActionExecutorError> {
        Err(DependencyActionExecutorError {
            message: "dependency installation is disabled in observer mode".into(),
        })
    }
}

#[allow(dead_code)]
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gent_drivers::installer::NpmGlobalPrefix;
    use gent_ports::{ApprovedPackageInstall, DependencyActionExecutor};

    use super::ObserverDependencyExecutor;

    #[test]
    fn public_provider_commands_are_fixed_and_shell_free() {
        let cases = [
            ("claude", "@anthropic-ai/claude-code"),
            ("claude", "@anthropic-ai/claude-code"),
            ("codex", "@openai/codex"),
            ("codex", "@openai/codex"),
        ];
        let npm = NpmGlobalPrefix::new(
            PathBuf::from("/app/node/npm"),
            PathBuf::from("/private/gentd/providers/npm-global"),
        );
        for (provider, package) in cases {
            let command = npm.pack(
                &ApprovedPackageInstall {
                    provider: provider.into(),
                    package_name: package.into(),
                    version: "1.2.3".into(),
                    integrity: "sha512-test".into(),
                    package_policy_digest_sha256: "a".repeat(64),
                },
                std::path::Path::new("/private/staging"),
            );
            assert_eq!(command.executable, "/app/node/npm");
            assert_eq!(command.arguments[5], format!("{package}@1.2.3"));
        }
    }

    #[test]
    fn observer_executor_never_starts_an_installer() {
        let error = ObserverDependencyExecutor
            .execute(&gent_ports::DependencyActionOperation {
                provider: "codex".into(),
                action: "install".into(),
            })
            .unwrap_err();
        assert!(error.message.contains("observer mode"));
    }
}
