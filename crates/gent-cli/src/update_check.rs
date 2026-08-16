//! Local, metadata-only update status with no daemon or release-source effects.

use clap::{Subcommand, ValueEnum};
use gent_types::{
    RuntimeReleaseChannel, RuntimeUpdateCheckReport, RuntimeUpdateCheckState, RuntimeUpdateFailure,
    RuntimeVersion,
};

/// Update commands intentionally stop at metadata-only status reporting.
#[derive(Debug, Subcommand)]
pub(crate) enum UpdateCommand {
    /// Read the local metadata-only runtime update status.
    Check {
        #[arg(long, value_enum, default_value_t = UpdateChannel::Stable)]
        channel: UpdateChannel,
    },
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

/// Reports this binary's version and the absence of a configured trusted source.
///
/// This intentionally does not connect to `gentd`, use the network, or inspect
/// an installer. It is a safe CLI affordance until an externally supervised
/// update source has an independently auditable trust contract.
#[must_use]
pub(crate) fn report(channel: RuntimeReleaseChannel) -> RuntimeUpdateCheckReport {
    RuntimeUpdateCheckReport {
        current_version: package_version(),
        channel,
        state: RuntimeUpdateCheckState::Unavailable,
        candidate: None,
        failure: Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable),
    }
}

fn package_version() -> RuntimeVersion {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let major = parse_component(parts.next());
    let minor = parse_component(parts.next());
    let patch = parse_component(parts.next());
    assert!(
        parts.next().is_none(),
        "package version must be major.minor.patch"
    );
    RuntimeVersion {
        major,
        minor,
        patch,
    }
}

fn parse_component(component: Option<&str>) -> u16 {
    component
        .and_then(|value| value.parse().ok())
        .expect("package version must use u16 numeric components")
}

#[cfg(test)]
mod tests {
    use gent_types::{RuntimeReleaseChannel, RuntimeUpdateCheckState, RuntimeUpdateFailure};

    use super::report;

    #[test]
    fn check_never_claims_a_trusted_release_or_contacts_a_daemon() {
        let result = report(RuntimeReleaseChannel::Canary);
        assert_eq!(result.channel, RuntimeReleaseChannel::Canary);
        assert_eq!(result.state, RuntimeUpdateCheckState::Unavailable);
        assert!(result.candidate.is_none());
        assert_eq!(
            result.failure,
            Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable)
        );
    }
}
