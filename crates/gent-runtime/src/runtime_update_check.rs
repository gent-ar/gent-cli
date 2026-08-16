//! Read-only assessment of a cached signed runtime release.
//!
//! This service deliberately has no ledger, coordinator, staging, process, or
//! activation dependency. A caller must opt in with an independently verified
//! release source before a report is available over local IPC.

use gent_core::{RuntimeUpdateContext, RuntimeUpdateEligibility, assess_runtime_update};
use gent_ports::runtime_update::RuntimeReleaseSource;
use gent_types::{
    RuntimeReleaseChannel, RuntimeUpdateCandidate, RuntimeUpdateCheckReport,
    RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeUpdateFailure, RuntimeVersion,
};

use crate::{RuntimeReleaseTrust, RuntimeReleaseTrustError};

/// Explicit boundary for a report-only cached update check.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeUpdateCheckAuthority {
    /// Shipped observer behavior: do not read a release source.
    #[default]
    Disabled,
    /// A caller supplied a release source that was validated during composition.
    CachedReadOnly,
}

/// Immutable local facts used to assess a release without daemon mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeUpdateCheckContext {
    pub current_version: RuntimeVersion,
    pub target: String,
    pub protocol: u16,
    pub schema: u32,
    pub app_version: RuntimeVersion,
    pub selected_cohort: bool,
}

/// Assesses a signed release without permitting an update effect.
#[derive(Clone, Debug)]
pub struct RuntimeUpdateCheckService<S> {
    source: S,
    trust: RuntimeReleaseTrust,
    context: RuntimeUpdateCheckContext,
    authority: RuntimeUpdateCheckAuthority,
}

impl<S> RuntimeUpdateCheckService<S> {
    /// Builds a read-only checker; callers retain control of the authority boundary.
    #[must_use]
    pub fn new(
        source: S,
        trust: RuntimeReleaseTrust,
        context: RuntimeUpdateCheckContext,
        authority: RuntimeUpdateCheckAuthority,
    ) -> Self {
        Self {
            source,
            trust,
            context,
            authority,
        }
    }
}

impl<S: RuntimeReleaseSource> RuntimeUpdateCheckService<S> {
    /// Returns a content-free report after revalidating the cached signed release.
    ///
    /// `now_unix_seconds` is supplied at request time so a daemon never serves a
    /// release cache past expiry merely because it has remained running.
    #[must_use]
    pub fn check(
        &self,
        request: RuntimeUpdateCheckRequest,
        now_unix_seconds: u64,
    ) -> RuntimeUpdateCheckReport {
        if self.authority != RuntimeUpdateCheckAuthority::CachedReadOnly {
            return unavailable(&self.context, request.channel, None);
        }
        let Ok(release) = self
            .source
            .fetch_release(request.channel, &self.context.target)
        else {
            return unavailable(&self.context, request.channel, None);
        };
        if release.payload.channel != request.channel
            || release.payload.artifact.target != self.context.target
        {
            return unavailable(
                &self.context,
                request.channel,
                Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable),
            );
        }
        if let Err(error) = self.trust.verify_release(&release, now_unix_seconds) {
            return unavailable(&self.context, request.channel, Some(trust_failure(&error)));
        }
        let eligibility = assess_runtime_update(
            &release.payload,
            RuntimeUpdateContext {
                protocol: self.context.protocol,
                schema: self.context.schema,
                app_version: self.context.app_version,
                selected_channel: request.channel,
                selected_cohort: self.context.selected_cohort,
                manifest_verified: true,
                now_unix_seconds,
            },
        );
        report_eligibility(
            &self.context,
            request.channel,
            &release.payload,
            eligibility,
        )
    }
}

fn report_eligibility(
    context: &RuntimeUpdateCheckContext,
    channel: RuntimeReleaseChannel,
    release: &gent_types::RuntimeReleaseManifest,
    eligibility: RuntimeUpdateEligibility,
) -> RuntimeUpdateCheckReport {
    match eligibility {
        RuntimeUpdateEligibility::Eligible if release.release_version > context.current_version => {
            RuntimeUpdateCheckReport {
                current_version: context.current_version,
                channel,
                state: RuntimeUpdateCheckState::Available,
                candidate: Some(RuntimeUpdateCandidate {
                    release_version: release.release_version,
                    artifact_digest_sha256: release.artifact.digest_sha256.clone(),
                    forward_only_schema: release.forward_only_schema,
                }),
                failure: None,
            }
        }
        RuntimeUpdateEligibility::Eligible | RuntimeUpdateEligibility::Deferred => {
            RuntimeUpdateCheckReport {
                current_version: context.current_version,
                channel,
                state: RuntimeUpdateCheckState::Current,
                candidate: None,
                failure: None,
            }
        }
        RuntimeUpdateEligibility::ReadOnlyUpdateRequired(failure) => RuntimeUpdateCheckReport {
            current_version: context.current_version,
            channel,
            state: RuntimeUpdateCheckState::ReadOnlyUpdateRequired,
            candidate: None,
            failure: Some(failure),
        },
        RuntimeUpdateEligibility::Rejected(failure) => unavailable(context, channel, Some(failure)),
    }
}

fn unavailable(
    context: &RuntimeUpdateCheckContext,
    channel: RuntimeReleaseChannel,
    failure: Option<RuntimeUpdateFailure>,
) -> RuntimeUpdateCheckReport {
    RuntimeUpdateCheckReport {
        current_version: context.current_version,
        channel,
        state: RuntimeUpdateCheckState::Unavailable,
        candidate: None,
        failure: failure.or(Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable)),
    }
}

fn trust_failure(error: &RuntimeReleaseTrustError) -> RuntimeUpdateFailure {
    match error {
        RuntimeReleaseTrustError::Expired => RuntimeUpdateFailure::Expired,
        RuntimeReleaseTrustError::RevokedRelease | RuntimeReleaseTrustError::RevokedSigner => {
            RuntimeUpdateFailure::Revoked
        }
        RuntimeReleaseTrustError::InvalidDigest => RuntimeUpdateFailure::InvalidArtifactDigest,
        RuntimeReleaseTrustError::InvalidSignature
        | RuntimeReleaseTrustError::UnknownSigner
        | RuntimeReleaseTrustError::Serialization(_)
        | RuntimeReleaseTrustError::UnsupportedManifestVersion
        | RuntimeReleaseTrustError::InvalidRollout
        | RuntimeReleaseTrustError::InvalidArtifact
        | RuntimeReleaseTrustError::InvalidCompatibilityRange => {
            RuntimeUpdateFailure::SignatureInvalid
        }
    }
}

#[cfg(test)]
#[path = "runtime_update_check_tests.rs"]
mod tests;
