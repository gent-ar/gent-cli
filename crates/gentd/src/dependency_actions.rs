//! Provider-specific installation commands with no shell interpolation.

use gent_drivers::installer::InstallerInvocation;
use gent_protocol::{DependencyAction, DependencyActionRequest, DependencyProvider};

/// Returns the exact vendor-supported command selected by an explicit request.
#[must_use]
pub(crate) fn invocation(request: &DependencyActionRequest) -> InstallerInvocation {
    let (executable, arguments) = match (request.provider, request.action) {
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

#[cfg(test)]
mod tests {
    use gent_protocol::{DependencyAction, DependencyActionRequest, DependencyProvider};

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
            let command = invocation(&DependencyActionRequest {
                provider,
                action,
                consent_granted: true,
            });
            assert_eq!(command.executable, executable);
            assert_eq!(command.arguments, arguments);
        }
    }
}
