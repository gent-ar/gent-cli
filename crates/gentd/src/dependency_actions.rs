//! Provider-specific installation commands with no shell interpolation.

use gent_drivers::installer::DependencyInstaller;
use gent_drivers::installer::InstallerInvocation;
use gent_ports::{
    DependencyActionExecutor, DependencyActionExecutorError, DependencyActionOperation,
};
use gent_protocol::{DependencyAction, DependencyProvider};

/// Returns the exact vendor-supported command selected by an explicit request.
#[must_use]
pub(crate) fn invocation(
    provider: DependencyProvider,
    action: DependencyAction,
) -> InstallerInvocation {
    let (executable, arguments) = match (provider, action) {
        (DependencyProvider::Claude, DependencyAction::Install) => (
            "npm",
            vec!["install", "--global", "@anthropic-ai/claude-code"],
        ),
        (DependencyProvider::Claude, DependencyAction::Update) => ("claude", vec!["update"]),
        (DependencyProvider::Codex, DependencyAction::Install) => {
            ("npm", vec!["install", "--global", "@openai/codex"])
        }
        (DependencyProvider::Codex, DependencyAction::Update) => ("codex", vec!["--upgrade"]),
    };
    InstallerInvocation {
        executable: executable.into(),
        arguments: arguments.into_iter().map(str::to_owned).collect(),
    }
}

/// Shell-free daemon adapter from the runtime action port to the public installer driver.
#[derive(Clone, Debug)]
pub(crate) struct SystemDependencyExecutor<I> {
    installer: I,
}

impl<I> SystemDependencyExecutor<I> {
    #[must_use]
    pub(crate) fn new(installer: I) -> Self {
        Self { installer }
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
        self.installer
            .execute(&invocation(provider, action))
            .map_err(|error| DependencyActionExecutorError {
                message: error.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use gent_protocol::{DependencyAction, DependencyProvider};

    use super::invocation;

    #[test]
    fn public_provider_commands_are_fixed_and_shell_free() {
        let cases = [
            (
                DependencyProvider::Claude,
                DependencyAction::Install,
                "npm",
                ["install", "--global", "@anthropic-ai/claude-code"].as_slice(),
            ),
            (
                DependencyProvider::Claude,
                DependencyAction::Update,
                "claude",
                ["update"].as_slice(),
            ),
            (
                DependencyProvider::Codex,
                DependencyAction::Install,
                "npm",
                ["install", "--global", "@openai/codex"].as_slice(),
            ),
            (
                DependencyProvider::Codex,
                DependencyAction::Update,
                "codex",
                ["--upgrade"].as_slice(),
            ),
        ];
        for (provider, action, executable, arguments) in cases {
            let command = invocation(provider, action);
            assert_eq!(command.executable, executable);
            assert_eq!(command.arguments, arguments);
        }
    }
}
