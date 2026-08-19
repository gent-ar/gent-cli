//! Read-only daemon composition for an exact current-run provider readiness reply.

use gent_ports::{AgentChatReadLedger, PackageInstallPolicy, ProvisionedProviderLockReader};
use gent_protocol::{
    DependencyProvider, ProviderReadinessFrame, ProviderReadinessReviewState,
    ProviderReadinessUnavailable,
};
use gent_runtime::AgentChatReadService;

use crate::private_provider_readiness::{
    PrivateProviderReadiness, PrivateProviderReadinessService,
};
use crate::{
    authority_clock::AuthorityClock, dependency_catalog::DependencyCatalog,
    private_provider_review::install_review,
};

/// Exact-review port exposed only by an explicitly composed private authority.
pub(crate) trait ProviderReadinessPort: Send + Sync {
    /// Assesses one current run without accepting provider or package input from the client.
    fn assess(&self, frame: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String>;
}

/// Maps durable current selection and private lock facts to content-safe readiness frames.
#[allow(dead_code)] // Kept unreachable until explicit evidence-bound authority composition.
#[derive(Clone, Debug)]
pub(crate) struct ProviderReadinessBoundary<L, P, C> {
    reads: AgentChatReadService<L>,
    readiness: PrivateProviderReadinessService<L>,
    catalog: DependencyCatalog,
    policy: P,
    clock: C,
}

#[allow(dead_code)] // Construction is intentionally reserved for explicit authority composition.
impl<L, P, C> ProviderReadinessBoundary<L, P, C> {
    #[must_use]
    pub(crate) fn new(
        reads: AgentChatReadService<L>,
        readiness: PrivateProviderReadinessService<L>,
        catalog: DependencyCatalog,
        policy: P,
        clock: C,
    ) -> Self {
        Self {
            reads,
            readiness,
            catalog,
            policy,
            clock,
        }
    }
}

#[allow(dead_code)] // Assessment is intentionally absent from default observer composition.
impl<L, P, C> ProviderReadinessBoundary<L, P, C>
where
    L: Clone + AgentChatReadLedger + ProvisionedProviderLockReader,
    P: PackageInstallPolicy,
    C: AuthorityClock,
{
    /// Assesses only the durable current run; stale selections must refresh before retrying.
    pub(crate) fn assess(
        &self,
        frame: ProviderReadinessFrame,
    ) -> Result<ProviderReadinessFrame, String> {
        let ProviderReadinessFrame::Assess {
            conversation_id,
            run_id,
        } = frame
        else {
            return Err("provider readiness response frames are server-only".into());
        };
        let detail = self
            .reads
            .detail(&conversation_id.0)
            .map_err(|error| error.to_string())?;
        if detail.current_run_id != run_id.0 {
            return Err(
                "staleAgentChatRun: refresh conversation detail before assessing readiness".into(),
            );
        }
        let selection = detail
            .runs
            .into_iter()
            .find(|run| run.run_id == run_id.0)
            .map(|run| run.selection)
            .ok_or_else(|| "agent-chat current run is absent from its conversation".to_owned())?;
        Ok(match self.readiness.assess(selection.provider) {
            PrivateProviderReadiness::Ready(_) => ProviderReadinessFrame::Ready {
                conversation_id,
                run_id,
                provider: selection.provider,
            },
            PrivateProviderReadiness::InstallReview => ProviderReadinessFrame::Review {
                conversation_id,
                run_id,
                state: ProviderReadinessReviewState::MissingInstall,
                review: self.review(selection.provider)?,
            },
            PrivateProviderReadiness::InvalidInstallation => ProviderReadinessFrame::Review {
                conversation_id,
                run_id,
                state: ProviderReadinessReviewState::InvalidInstallation,
                review: self.review(selection.provider)?,
            },
            PrivateProviderReadiness::ClaurstUnavailable => ProviderReadinessFrame::Unavailable {
                conversation_id,
                run_id,
                reason: ProviderReadinessUnavailable::ClaurstPrivateBridge,
            },
            PrivateProviderReadiness::Unavailable => ProviderReadinessFrame::Unavailable {
                conversation_id,
                run_id,
                reason: ProviderReadinessUnavailable::ProvenanceUnreadable,
            },
        })
    }

    fn review(
        &self,
        provider: gent_types::AgentChatProvider,
    ) -> Result<gent_protocol::ProviderInstallReview, String> {
        let provider = match provider {
            gent_types::AgentChatProvider::Claude => DependencyProvider::Claude,
            gent_types::AgentChatProvider::Codex => DependencyProvider::Codex,
            gent_types::AgentChatProvider::Claurst => {
                return Err("Claurst cannot receive a public npm install review".into());
            }
        };
        install_review(&self.catalog, &self.policy, &self.clock, provider)
    }
}

impl<L, P, C> ProviderReadinessPort for ProviderReadinessBoundary<L, P, C>
where
    L: Clone + AgentChatReadLedger + ProvisionedProviderLockReader + Send + Sync,
    P: PackageInstallPolicy + Send + Sync,
    C: AuthorityClock + Send + Sync,
{
    fn assess(&self, frame: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String> {
        Self::assess(self, frame)
    }
}
