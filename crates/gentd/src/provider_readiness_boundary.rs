//! Read-only daemon composition for an exact current-run provider readiness reply.

use gent_ports::{AgentChatReadLedger, ProvisionedProviderLockReader};
use gent_protocol::{
    ProviderReadinessFrame, ProviderReadinessReviewState, ProviderReadinessUnavailable,
};
use gent_runtime::AgentChatReadService;

use crate::private_provider_readiness::{
    PrivateProviderReadiness, PrivateProviderReadinessService,
};

/// Maps durable current selection and private lock facts to content-safe readiness frames.
#[derive(Clone, Debug)]
pub(crate) struct ProviderReadinessBoundary<L> {
    reads: AgentChatReadService<L>,
    readiness: PrivateProviderReadinessService<L>,
}

impl<L> ProviderReadinessBoundary<L> {
    #[must_use]
    pub(crate) fn new(
        reads: AgentChatReadService<L>,
        readiness: PrivateProviderReadinessService<L>,
    ) -> Self {
        Self { reads, readiness }
    }
}

impl<L: Clone + AgentChatReadLedger + ProvisionedProviderLockReader> ProviderReadinessBoundary<L> {
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
            PrivateProviderReadiness::InstallReview(plan) => ProviderReadinessFrame::Review {
                conversation_id,
                run_id,
                state: ProviderReadinessReviewState::MissingInstall,
                plan,
            },
            PrivateProviderReadiness::InvalidInstallation(plan) => ProviderReadinessFrame::Review {
                conversation_id,
                run_id,
                state: ProviderReadinessReviewState::InvalidInstallation,
                plan,
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
}
