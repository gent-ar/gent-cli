//! Provider-specific installation commands with no shell interpolation.

use gent_drivers::installer::{DependencyInstaller, InstallerInvocation, NpmGlobalPrefix};
use gent_ports::{
    DependencyActionExecutor, DependencyActionExecutorError, DependencyActionOperation,
};
use gent_protocol::{DependencyAction, DependencyProvider};

/// Returns the exact vendor-supported command selected by an explicit request.
#[must_use]
pub(crate) fn invocation(
    npm: &NpmGlobalPrefix,
    provider: DependencyProvider,
    _: DependencyAction,
) -> InstallerInvocation {
    npm.install(match provider {
        DependencyProvider::Claude => "@anthropic-ai/claude-code",
        DependencyProvider::Codex => "@openai/codex",
    })
}

/// Shell-free daemon adapter from the runtime action port to the public installer driver.
#[derive(Clone, Debug)]
pub(crate) struct SystemDependencyExecutor<I> {
    installer: I,
    npm: Option<NpmGlobalPrefix>,
}

impl<I> SystemDependencyExecutor<I> {
    #[must_use]
    pub(crate) fn new(installer: I, npm: Option<NpmGlobalPrefix>) -> Self {
        Self { installer, npm }
    }
}

impl<I: DependencyInstaller> DependencyActionExecutor for SystemDependencyExecutor<I> {
    fn execute(
        &self,
        operation: &DependencyActionOperation,
    ) -> Result<(), DependencyActionExecutorError> {
        let provider =
            operation
                .provider
                .parse()
                .map_err(
                    |error: gent_protocol::ProtocolError| DependencyActionExecutorError {
                        message: error.to_string(),
                    },
                )?;
        let action = operation
            .action
            .parse()
            .map_err(
                |error: gent_protocol::ProtocolError| DependencyActionExecutorError {
                    message: error.to_string(),
                },
            )?;
        let npm = self
            .npm
            .as_ref()
            .ok_or_else(|| DependencyActionExecutorError {
                message: "bundled Node runtime is unavailable; set GENT_NODE_BINARY".into(),
            })?;
        self.installer
            .execute(&invocation(npm, provider, action))
            .map_err(|error| DependencyActionExecutorError {
                message: error.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gent_drivers::installer::NpmGlobalPrefix;
    use gent_protocol::{DependencyAction, DependencyProvider};

    use super::invocation;

    #[test]
    fn public_provider_commands_are_fixed_and_shell_free() {
        let cases = [
            (
                DependencyProvider::Claude,
                DependencyAction::Install,
                "@anthropic-ai/claude-code",
            ),
            (
                DependencyProvider::Claude,
                DependencyAction::Update,
                "@anthropic-ai/claude-code",
            ),
            (
                DependencyProvider::Codex,
                DependencyAction::Install,
                "@openai/codex",
            ),
            (
                DependencyProvider::Codex,
                DependencyAction::Update,
                "@openai/codex",
            ),
        ];
        let npm = NpmGlobalPrefix::new(
            PathBuf::from("/app/node/npm"),
            PathBuf::from("/private/gentd/providers/npm-global"),
        );
        for (provider, action, package) in cases {
            let command = invocation(&npm, provider, action);
            assert_eq!(command.executable, "/app/node/npm");
            assert_eq!(command.arguments[3], "/private/gentd/providers/npm-global");
            assert_eq!(command.arguments[4], package);
        }
    }
}
