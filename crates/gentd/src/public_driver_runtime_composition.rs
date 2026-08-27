use super::{PublicDriversRuntime, PublicDriversRuntimeError};
use crate::{
    authority_clock::SystemAuthorityClock, authority_profile::ValidatedAuthorityProfile,
    compatibility_assessment::CompatibilityAssessment,
    fresh_compatibility_authorizer::FreshCompatibilityAuthorizer,
    provider_effects::ProviderEffectDispatcher,
};
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    PublicProviderRunner, RunVersionAuthorizer, TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchAuthority, AgentChatPromptDispatchService, AgentChatReadService,
    AgentChatTranscriptAuthority, AgentChatTranscriptIngress, ConversationActivityAuthority,
    ConversationActivityService, Coordinator, ProviderActivityIngress, ProviderRunAuthority,
    PublicRunService,
};
use std::sync::Arc;
#[derive(Clone, Debug)]
pub(crate) enum DriverCompatibilityAuthorizer {
    Bootstrap(CompatibilityAssessment),
    Fresh(FreshCompatibilityAuthorizer<SystemAuthorityClock>),
    /// Explicit local executable locks used by the standalone Gent profile. This is intentionally
    /// narrower than the release-backed authorizers: only locks captured by the local resolver
    /// with the fixed marker can reach a provider launch.
    StandaloneLocal,
}
impl RunVersionAuthorizer for DriverCompatibilityAuthorizer {
    fn authorize(
        &self,
        lock: &gent_types::RunVersionLock,
    ) -> Result<(), gent_ports::PublicProviderRunError> {
        match self {
            Self::Bootstrap(a) => a.authorize(lock),
            Self::Fresh(a) => a.authorize(lock),
            Self::StandaloneLocal
                if lock.compatibility_entry == "standalone-local-v1"
                    && !lock.provider.is_empty()
                    && lock.digest_sha256.len() == 64 =>
            {
                Ok(())
            }
            Self::StandaloneLocal => Err(gent_ports::PublicProviderRunError::CompatibilityDenied),
        }
    }
}
impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: PublicProviderRunner + Clone,
    R: PublicProviderResolver,
{
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn new(
        profile: ValidatedAuthorityProfile,
        coordinator: Coordinator<L>,
        ledger: L,
        compatibility: CompatibilityAssessment,
        runner: D,
        resolver: R,
    ) -> Result<Self, PublicDriversRuntimeError> {
        let authorizer = DriverCompatibilityAuthorizer::Bootstrap(compatibility.clone());
        Self::build(
            profile,
            coordinator,
            ledger,
            Some(&compatibility),
            authorizer,
            runner,
            resolver,
        )
    }
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn new_with_current_compatibility(
        profile: ValidatedAuthorityProfile,
        coordinator: Coordinator<L>,
        ledger: L,
        compatibility: CompatibilityAssessment,
        runner: D,
        resolver: R,
    ) -> Result<Self, PublicDriversRuntimeError> {
        Self::build(
            profile,
            coordinator,
            ledger,
            Some(&compatibility),
            DriverCompatibilityAuthorizer::Fresh(FreshCompatibilityAuthorizer::new(
                compatibility.clone(),
                SystemAuthorityClock,
            )),
            runner,
            resolver,
        )
    }
    /// Builds the real public-driver runtime from explicit locally captured executables.
    ///
    /// The caller still has to provide a prepared public-driver profile and a resolver that
    /// rechecks each executable identity immediately before launch. Unlike the packaged
    /// authority path this does not require a compatibility-manifest artifact.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn new_standalone_local(
        profile: ValidatedAuthorityProfile,
        coordinator: Coordinator<L>,
        ledger: L,
        runner: D,
        resolver: R,
    ) -> Result<Self, PublicDriversRuntimeError> {
        Self::build(
            profile,
            coordinator,
            ledger,
            None,
            DriverCompatibilityAuthorizer::StandaloneLocal,
            runner,
            resolver,
        )
    }
    fn build(
        profile: ValidatedAuthorityProfile,
        coordinator: Coordinator<L>,
        ledger: L,
        compatibility: Option<&CompatibilityAssessment>,
        authorizer: DriverCompatibilityAuthorizer,
        runner: D,
        resolver: R,
    ) -> Result<Self, PublicDriversRuntimeError> {
        let ValidatedAuthorityProfile::PreparedPublicDrivers(approval) = profile else {
            return Err(PublicDriversRuntimeError::ObserverProfile);
        };
        if let Some(compatibility) = compatibility {
            let Some(actual_digest) = compatibility.manifest_sha256() else {
                return Err(PublicDriversRuntimeError::CompatibilityManifestUnavailable);
            };
            if actual_digest != approval.compatibility_manifest_sha256 {
                return Err(PublicDriversRuntimeError::CompatibilityManifestMismatch);
            }
        }
        Ok(Self {
            ledger: ledger.clone(),
            runs: Arc::new(PublicRunService::new(
                coordinator.clone(),
                runner.clone(),
                authorizer,
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
            transcripts: AgentChatTranscriptIngress::new(
                ledger.clone(),
                AgentChatTranscriptAuthority::Approved,
            ),
            dispatches: AgentChatPromptDispatchService::new(
                ledger.clone(),
                AgentChatPromptDispatchAuthority::Approved,
            ),
            reads: AgentChatReadService::new(ledger.clone()),
            contexts: super::context::RunContextProjection::new(ledger),
            goal_resolver: None,
            attachment_root: None,
            codex_attachment_root: None,
        })
    }
}
