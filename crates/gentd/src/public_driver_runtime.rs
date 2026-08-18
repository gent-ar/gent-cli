//! Dormant, separately approved public-driver composition edge; `main` never constructs it.
use crate::authority_profile::ValidatedAuthorityProfile;
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::provider_effects::ProviderEffectDispatcher;
use gent_drivers::{SessionEffect, public_protocol::PublicWireFact};
use gent_ports::{
    ActiveGoalResolver, AgentChatPromptDispatchLedger, AgentChatReadLedger,
    ConversationActivityLedger, Ledger, PublicProviderResolver, PublicProviderRunner,
    RunProjectionLedger, TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchAuthority, AgentChatPromptDispatchResult,
    AgentChatPromptDispatchService, AgentChatReadService, AgentChatTranscriptAppendRequest,
    AgentChatTranscriptAppendResult, AgentChatTranscriptAuthority, AgentChatTranscriptIngress,
    ConversationActivityAuthority, ConversationActivityResult, ConversationActivityService,
    Coordinator, ProviderActivityFact, ProviderActivityIngress, ProviderRunAuthority,
    PublicRunService, RuntimeError,
};
use gent_types::{AgentChatProvider, AgentChatSelection, GoalProjection, HostEpoch, RunLiveStatus};
use std::sync::Arc;

/// A fact emitted at the public-driver process boundary with its own durable source identity.
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PublicDriverFactResult {
    Lifecycle(Option<RunLiveStatus>),
    Activity(ConversationActivityResult),
    Transcript(AgentChatTranscriptAppendResult),
}
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PublicDriversRuntimeError {
    #[error("the observer profile cannot construct public-driver authority")]
    ObserverProfile,
    #[error("the approved compatibility manifest is unavailable")]
    CompatibilityManifestUnavailable,
    #[error("the approved compatibility manifest digest does not match the verified cache")]
    CompatibilityManifestMismatch,
}
/// A fully injected, authority-gated runtime deliberately absent from `RuntimeFacade`.
#[derive(Debug)]
pub(crate) struct PublicDriversRuntime<L, D, R> {
    runs: Arc<PublicRunService<L, D, CompatibilityAssessment, R>>,
    runner: D,
    effects: ProviderEffectDispatcher<L>,
    activity: ProviderActivityIngress<L>,
    transcripts: AgentChatTranscriptIngress<L>,
    dispatches: AgentChatPromptDispatchService<L>,
    reads: AgentChatReadService<L>,
    pub(crate) contexts: context::RunContextProjection<L>,
    goal_resolver: Option<Arc<dyn ActiveGoalResolver>>,
}
impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: PublicProviderRunner + Clone,
    R: PublicProviderResolver,
{
    /// Binds a validated approved profile to one verified compatibility envelope and ports.
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
                runner.clone(),
                compatibility,
                resolver,
                ProviderRunAuthority::PublicDrivers,
            )),
            runner,
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
                ledger.clone(),
                AgentChatPromptDispatchAuthority::Approved,
            ),
            reads: AgentChatReadService::new(ledger.clone()),
            contexts: context::RunContextProjection::new(ledger),
            goal_resolver: None,
        })
    }

    #[must_use]
    pub(crate) fn with_active_goal_resolver(
        mut self,
        resolver: Arc<dyn ActiveGoalResolver>,
    ) -> Self {
        self.goal_resolver = Some(resolver);
        self
    }

    /// Resolves fresh goal context immediately before a daemon-owned provider turn.
    pub(crate) fn active_goal_for(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<GoalProjection>, RuntimeError> {
        self.goal_resolver.as_ref().map_or(Ok(None), |resolver| {
            resolver
                .resolve_active_goal(conversation_id, run_id)
                .map_err(RuntimeError::from)
        })
    }

    /// Returns the only process lifecycle service constructed by this authority profile.
    #[must_use]
    pub(crate) fn runs(&self) -> &Arc<PublicRunService<L, D, CompatibilityAssessment, R>> {
        &self.runs
    }

    /// Returns the exact clone shared with the durable public-run service.
    #[must_use]
    pub(crate) fn runner(&self) -> D {
        self.runner.clone()
    }

    /// Persists one runner-owned source fact through its matching durable ingress.
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

    /// Marks the durable boundary before a provider process may be launched.
    ///
    /// # Errors
    /// Returns an error when durable ownership validation rejects the release.
    pub(crate) fn begin_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .begin_launch(message_id, coordinator_id, host_epoch)
    }

    /// Confirms that the daemon-owned runner successfully launched the provider process.
    ///
    /// # Errors
    /// Returns an error when durable ownership validation rejects settlement.
    pub(crate) fn confirm_prompt_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .confirm_started(message_id, coordinator_id, host_epoch)
    }

    /// Returns a claim to the durable outbox before a launch boundary is crossed.
    pub(crate) fn release_prompt_claim(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .release_claim(message_id, coordinator_id, host_epoch)
    }

    /// Returns a launch marker only after a local result proves no provider runner was called.
    pub(crate) fn release_unstarted_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .release_unstarted_launch(message_id, coordinator_id, host_epoch)
    }

    /// Retires an ambiguous launch without allowing the durable prompt to replay automatically.
    pub(crate) fn mark_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .mark_unprovable(message_id, coordinator_id, host_epoch)
    }

    /// Settles one terminal provider prompt through its durable outbox owner.
    pub(crate) fn settle_prompt(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .settle(message_id, coordinator_id, host_epoch)
    }

    /// Recovers only pre-launch work after a successor daemon has fenced the previous epoch.
    pub(crate) fn recover_prompts(&self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        self.dispatches.recover(host_epoch)
    }
}
impl<L: AgentChatReadLedger, D, R> PublicDriversRuntime<L, D, R> {
    pub(crate) fn selection_for_run(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<AgentChatSelection, RuntimeError> {
        self.reads.run_selection(conversation_id, run_id)
    }
}

mod context;
