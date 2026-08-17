//! Local, metadata-only update status with no daemon or release-source effects.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use gent_types::RuntimeReleaseChannel;

/// Update commands intentionally stop at metadata-only status reporting.
#[derive(Debug, Subcommand)]
pub(crate) enum UpdateCommand {
    /// Run, inspect, enable, or disable the external paired-runtime scheduler.
    Auto {
        #[command(subcommand)]
        action: AutoUpdateAction,
    },
    /// Read one durable update planning or successor-recovery maintenance record.
    Status {
        #[arg(long)]
        attempt_id: String,
    },
    /// Read the local metadata-only runtime update status.
    Check {
        #[arg(long, value_enum, default_value_t = UpdateChannel::Stable)]
        channel: UpdateChannel,
    },
    /// Verify a tag-pinned installer, then explicitly update an idle runtime pair.
    Apply {
        /// Published release tag to install; no implicit "latest" selection is allowed.
        #[arg(long, value_parser = release_version)]
        version: String,
        /// SHA-256 from the signed target archive manifest, used as an explicit confirmation.
        #[arg(long, value_parser = archive_digest)]
        expected_sha256: String,
        /// Required acknowledgement before downloading or activating a new release pair.
        #[arg(long)]
        consent: bool,
        /// Override the installation root passed to the signed external installer.
        #[arg(long)]
        install_dir: Option<PathBuf>,
    },
}

/// Automatic updates are external and only operate on an installed runtime pair.
#[derive(Debug, Subcommand)]
pub(crate) enum AutoUpdateAction {
    Enable {
        #[arg(long, default_value_t = 6 * 60 * 60)]
        interval_seconds: u32,
    },
    Disable,
    Status,
    Run {
        #[arg(long)]
        force: bool,
    },
}

impl AutoUpdateAction {
    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Enable { .. } => "enable",
            Self::Disable => "disable",
            Self::Status => "status",
            Self::Run { .. } => "run",
        }
    }

    pub(crate) const fn interval_seconds(&self) -> Option<u32> {
        match self {
            Self::Enable { interval_seconds } => Some(*interval_seconds),
            Self::Disable | Self::Status | Self::Run { .. } => None,
        }
    }

    pub(crate) const fn force(&self) -> bool {
        matches!(self, Self::Run { force: true })
    }
}

/// Explicit selection for a future trusted release channel.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum UpdateChannel {
    Stable,
    Beta,
    Canary,
}

impl From<UpdateChannel> for RuntimeReleaseChannel {
    fn from(channel: UpdateChannel) -> Self {
        match channel {
            UpdateChannel::Stable => Self::Stable,
            UpdateChannel::Beta => Self::Beta,
            UpdateChannel::Canary => Self::Canary,
        }
    }
}

fn release_version(value: &str) -> Result<String, String> {
    let numeric = value
        .strip_prefix('v')
        .ok_or("release version must start with v")?;
    let mut parts = numeric.split('.');
    let valid = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.chars().all(char::is_numeric))
    }) && parts.next().is_none();
    valid
        .then(|| value.to_owned())
        .ok_or("release version must be vMAJOR.MINOR.PATCH".into())
}

fn archive_digest(value: &str) -> Result<String, String> {
    (value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then(|| value.to_owned())
    .ok_or("expected SHA-256 must be 64 lowercase hexadecimal characters".into())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{AutoUpdateAction, UpdateCommand};

    #[test]
    fn release_identity_and_digest_parsers_fail_closed() {
        assert!(super::release_version("v1.2.3").is_ok());
        assert!(super::release_version("1.2.3").is_err());
        assert!(super::release_version("v1.2.3-beta").is_err());
        assert!(super::archive_digest(&"a".repeat(64)).is_ok());
        assert!(super::archive_digest(&"A".repeat(64)).is_err());
    }

    #[test]
    fn automatic_update_requires_an_explicit_subcommand() {
        #[derive(Parser)]
        struct Command {
            #[command(subcommand)]
            action: UpdateCommand,
        }
        let command =
            Command::try_parse_from(["gent", "auto", "enable", "--interval-seconds", "600"])
                .unwrap();
        assert!(matches!(
            command.action,
            UpdateCommand::Auto {
                action: AutoUpdateAction::Enable {
                    interval_seconds: 600
                }
            }
        ));
    }
}
