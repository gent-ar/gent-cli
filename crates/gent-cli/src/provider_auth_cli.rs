//! Terminal requests for the secret-free provider-auth protocol extension.

use clap::{Subcommand, ValueEnum};
use std::{path::PathBuf, process::Command};

/// Terminal-only provider authentication actions.
#[derive(Debug, Subcommand)]
pub(crate) enum ProviderAuthCommand {
    /// Install if needed and start the provider's own interactive login flow.
    Login { provider: ProviderArgument },
}

/// Public providers supported by the secret-free authentication contract.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProviderArgument {
    Claude,
    Codex,
}

pub(crate) fn login_interactive(
    data_dir: Option<PathBuf>,
    provider: ProviderArgument,
) -> Result<String, String> {
    let data_dir = data_dir.unwrap_or_else(crate::local_ipc::default_data_dir);
    let daemon = std::env::var_os("GENTD_BIN")
        .map_or_else(crate::local_ipc::default_daemon_binary, PathBuf::from);
    let provider = match provider {
        ProviderArgument::Claude => "claude",
        ProviderArgument::Codex => "codex",
    };
    let status = Command::new(daemon)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--provider-login")
        .arg(provider)
        .status()
        .map_err(|error| format!("Gent could not start the {provider} login flow: {error}"))?;
    status
        .success()
        .then(|| format!("{provider} login completed. You can continue this conversation."))
        .ok_or_else(|| format!("{provider} login did not complete."))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::ProviderAuthCommand;
    use crate::{Args, CommandLine};

    #[test]
    fn parses_public_provider_login_command() {
        let login = Args::try_parse_from(["gent", "auth", "login", "codex"]).unwrap();
        assert!(matches!(
            login.command,
            Some(CommandLine::Auth {
                action: ProviderAuthCommand::Login { .. }
            })
        ));
    }
}
