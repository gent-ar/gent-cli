//! Dormant composition edge for a separately approved public-driver host.
//!
//! `main` never constructs this type. It exists so the future authority handoff has one typed
//! place to bind an approved profile, a signed compatibility envelope, process ports, lifecycle
//! facts, and activity facts without adding an alternate startup switch to the observer daemon.

use std::sync::Arc;

use gent_drivers::{SessionEffect, public_protocol::PublicWireFact};
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    PublicProviderRunner, RunProjectionLedger, TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchAuthority, AgentChatPromptDispatchResult,
    AgentChatPromptDispatchService, AgentChatTranscriptAppendRequest,
    AgentChatTranscriptAppendResult, AgentChatTranscriptAuthority, AgentChatTranscriptIngress,
    ConversationActivityAuthority, ConversationActivityResult, ConversationActivityService,
    Coordinator, ProviderActivityFact, ProviderActivityIngress, ProviderRunAuthority,
    PublicRunService, RuntimeError,
};
use gent_types::{AgentChatProvider, HostEpoch, RunLiveStatus};

use crate::authority_profile::ValidatedAuthorityProfile;
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::provider_effects::ProviderEffectDispatcher;

/// A fact emitted at the public-driver process boundary.
///
/// Each variant keeps its own durable source identity. Activity facts cannot share an event ID
/// with lifecycle facts because they have distinct source schemas and idempotency contracts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PublicDriverFact {
    SessionEffect {
        event_id: String,
        effect: SessionEffect,
    },
    PublicWire {
        event_id: String,
        fact: PublicWireFact,
    },
    Activity(ProviderActivityFact),
    /// A daemon-mapped transcript fact. The daemon, not the driver, supplies durable IDs.
    Transcript(AgentChatTranscriptAppendRequest),
}

/// Result of persisting a public-driver fact through its typed ingress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PublicDriverFactResult {
    Lifecycle(Option<RunLiveStatus>),
    Activity(ConversationActivityResult),
    Transcript(AgentChatTranscriptAppendResult),
}

/// Refuses authority composition before profile and compatibility bindings agree.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PublicDriversRuntimeError {
    #[error("the observer profile cannot construct public-driver authority")]
    ObserverProfile,
    #[error("the approved compatibility manifest is unavailable")]
    CompatibilityManifestUnavailable,
    #[error("the approved compatibility manifest digest does not match the verified cache")]
    CompatibilityManifestMismatch,
}

/// A fully injected, authority-gated public-driver runtime.
///
/// It is deliberately not a `RuntimeFacade`: the shipped facade uses the hard observer service.
/// A reviewed future composition must explicitly select this value after all evidence gates pass.
#[derive(Debug)]
pub(crate) struct PublicDriversRuntime<L, D, R> {
    runs: Arc<PublicRunService<L, D, CompatibilityAssessment, R>>,
    effects: ProviderEffectDispatcher<L>,
    activity: ProviderActivityIngress<L>,
    transcripts: AgentChatTranscriptIngress<L>,
    dispatches: AgentChatPromptDispatchService<L>,
}

impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: PublicProviderRunner,
    R: PublicProviderResolver,
{
    /// Binds only a validated approved profile to one verified compatibility envelope and ports.
    ///
    /// # Errors
    /// Returns an error before resolver or runner construction becomes reachable when the profile
    /// is observer-only or its immutable approved digest differs from verified local material.
    pub(crate) fn new(
        profile: ValidatedAuthorityProfile,
        coordinator: Coordinator<L>,
        ledger: L,
        compatibility: CompatibilityAssessment,
        runner: D,
        resolver: R,
    ) -> Result<Self, PublicDriversRuntimeError> {
        let approval = match profile {
            ValidatedAuthorityProfile::PreparedPublicDrivers(approval)
            | ValidatedAuthorityProfile::PreparedPublicDriversAndMcp {
                public_drivers: approval,
                ..
            } => approval,
            ValidatedAuthorityProfile::Observer | ValidatedAuthorityProfile::PreparedMcp(_) => {
                return Err(PublicDriversRuntimeError::ObserverProfile);
            }
        };
        let Some(actual_digest) = compatibility.manifest_sha256() else {
            return Err(PublicDriversRuntimeError::CompatibilityManifestUnavailable);
        };
        if actual_digest != approval.compatibility_manifest_sha256 {
            return Err(PublicDriversRuntimeError::CompatibilityManifestMismatch);
        }
        Ok(Self {
            runs: Arc::new(PublicRunService::new(
                coordinator.clone(),
                runner,
                compatibility,
                resolver,
                ProviderRunAuthority::PublicDrivers,
            )),
            effects: ProviderEffectDispatcher::new(
                coordinator.clone(),
                ProviderRunAuthority::PublicDrivers,
            ),
            activity: ProviderActivityIngress::new(
                coordinator,
                ConversationActivityService::new(
                    ledger.clone(),
                    ConversationActivityAuthority::Approved,
                ),
                ProviderRunAuthority::PublicDrivers,
            ),
            // This composition is unreachable from the observer daemon. A runner must map each
            // provider fact to the durable prompt turn before it may call this ingress.
            transcripts: AgentChatTranscriptIngress::new(
                ledger.clone(),
                AgentChatTranscriptAuthority::Approved,
            ),
            dispatches: AgentChatPromptDispatchService::new(
                ledger,
                AgentChatPromptDispatchAuthority::Approved,
            ),
        })
    }

    /// Returns the only process lifecycle service constructed by this authority profile.
    #[must_use]
    pub(crate) fn runs(&self) -> &Arc<PublicRunService<L, D, CompatibilityAssessment, R>> {
        &self.runs
    }

    /// Persists one runner-owned source fact through its matching durable ingress.
    ///
    /// # Errors
    /// Returns an error when the run lease, epoch fence, session ordering, or activity projection
    /// rejects the owned fact.
    pub(crate) fn record(
        &self,
        run_id: String,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        fact: PublicDriverFact,
    ) -> Result<PublicDriverFactResult, RuntimeError> {
        match fact {
            PublicDriverFact::SessionEffect { event_id, effect } => self
                .effects
                .record(event_id, run_id, coordinator_id, host_epoch, &effect)
                .map(PublicDriverFactResult::Lifecycle),
            PublicDriverFact::PublicWire { event_id, fact } => self
                .effects
                .record_public_wire_fact(event_id, run_id, coordinator_id, host_epoch, &fact)
                .map(PublicDriverFactResult::Lifecycle),
            PublicDriverFact::Activity(activity) => self
                .activity
                .record(coordinator_id, activity)
                .map(PublicDriverFactResult::Activity),
            PublicDriverFact::Transcript(append) => self
                .transcripts
                .append(&append)
                .map(PublicDriverFactResult::Transcript),
        }
    }

    /// Claims the next durable prompt only for this approved daemon lifecycle owner.
    ///
    /// # Errors
    /// Returns an error when the durable ownership fence rejects the claim.
    pub(crate) fn claim_prompt(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<AgentChatPromptDispatchResult, RuntimeError> {
        self.dispatches.claim(coordinator_id, host_epoch, provider)
    }
}
