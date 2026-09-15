use gent_protocol::{
    DependencyProvider, ProviderReadinessFrame, ProviderReadinessReviewState,
    ProviderReadinessUnavailable,
};
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::AgentChatProvider;

use crate::{
    ordinary_authority_release::OrdinaryAuthorityReleaseError,
    private_provider_readiness::PrivateProviderReadiness,
    provider_executables::ProviderExecutables, provider_readiness_boundary::ProviderReadinessPort,
    standalone_authority_composition::StandaloneClaurstModels,
};

pub(crate) trait StandalonePublicProviderReadiness: Send + Sync + std::fmt::Debug {
    fn is_ready(&self, provider: AgentChatProvider) -> Result<bool, String>;
}

pub(crate) type StandalonePublicProviderReadinessPort =
    std::sync::Arc<dyn StandalonePublicProviderReadiness>;

#[derive(Clone, Debug)]
pub(crate) struct StandaloneProviderReadinessAuthority {
    reads: AgentChatReadService<SqliteLedger>,
    executables: ProviderExecutables,
    local_models: StandaloneClaurstModels,
}

impl StandaloneProviderReadinessAuthority {
    pub(crate) fn new(
        ledger: SqliteLedger,
        executables: ProviderExecutables,
        local_models: StandaloneClaurstModels,
    ) -> Self {
        Self {
            reads: AgentChatReadService::new(ledger),
            executables,
            local_models,
        }
    }

    fn explicit_readiness(&self, provider: AgentChatProvider) -> Option<bool> {
        self.executables
            .explicit_path(provider)
            .map(std::path::Path::is_file)
    }

    fn release_unavailable(&self) -> Option<ProviderReadinessUnavailable> {
        let error = self
            .executables
            .release()?
            .load(crate::startup::unix_seconds())
            .err()?;
        Some(match error {
            OrdinaryAuthorityReleaseError::RuntimeUnverified => {
                ProviderReadinessUnavailable::RuntimeUnverified
            }
            _ => ProviderReadinessUnavailable::ProvenanceUnreadable,
        })
    }

    fn review(
        &self,
        provider: AgentChatProvider,
    ) -> Result<gent_protocol::ProviderInstallReview, String> {
        let provider = match provider {
            AgentChatProvider::Claude => DependencyProvider::Claude,
            AgentChatProvider::Codex => DependencyProvider::Codex,
            AgentChatProvider::Claurst => {
                return Err("Claurst cannot enter public provider provisioning".into());
            }
        };
        crate::private_provider_review::install_review(
            &crate::dependency_catalog::DependencyCatalog::default(),
            self.executables
                .release()
                .ok_or_else(|| "signed provider release is unavailable".to_owned())?,
            &crate::authority_clock::SystemAuthorityClock,
            provider,
        )
    }
}

impl StandalonePublicProviderReadiness for StandaloneProviderReadinessAuthority {
    fn is_ready(&self, provider: AgentChatProvider) -> Result<bool, String> {
        Ok(self.explicit_readiness(provider).unwrap_or_else(|| {
            matches!(
                self.executables.installed_readiness(provider),
                PrivateProviderReadiness::Ready(_)
            )
        }))
    }
}

impl ProviderReadinessPort for StandaloneProviderReadinessAuthority {
    fn assess(&self, frame: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String> {
        let ProviderReadinessFrame::Assess {
            conversation_id,
            run_id,
        } = frame
        else {
            return Err("provider readiness reply frames are server-only".into());
        };
        let detail = self
            .reads
            .detail(&conversation_id.0)
            .map_err(|error| error.to_string())?;
        if detail.current_run_id != run_id.0 {
            return Err("provider readiness requires the current durable run".into());
        }
        let selection = detail
            .runs
            .into_iter()
            .find(|run| run.run_id == run_id.0)
            .map(|run| run.selection)
            .ok_or_else(|| "provider readiness run is absent".to_owned())?;
        let provider = selection.provider;
        if provider == AgentChatProvider::Claurst {
            let install = self
                .local_models
                .install_state(&selection.model)
                .map_err(|error| error.to_string())?;
            return Ok(match install {
                gent_protocol::LocalModelInstallState::Ready { .. } => {
                    ProviderReadinessFrame::Ready {
                        conversation_id,
                        run_id,
                        provider,
                    }
                }
                install => ProviderReadinessFrame::LocalModel {
                    conversation_id,
                    run_id,
                    model_id: selection.model,
                    install,
                },
            });
        }
        let readiness = self.executables.installed_readiness(provider);
        let explicit = self.explicit_readiness(provider);
        Ok(
            if explicit == Some(true)
                || (explicit.is_none() && matches!(readiness, PrivateProviderReadiness::Ready(_)))
            {
                ProviderReadinessFrame::Ready {
                    conversation_id,
                    run_id,
                    provider,
                }
            } else if let Some(reason) = explicit
                .is_none()
                .then(|| self.release_unavailable())
                .flatten()
            {
                ProviderReadinessFrame::Unavailable {
                    conversation_id,
                    run_id,
                    reason,
                }
            } else if self.executables.release().is_some()
                && (explicit == Some(false)
                    || matches!(
                        readiness,
                        PrivateProviderReadiness::InstallReview
                            | PrivateProviderReadiness::InvalidInstallation
                    ))
            {
                ProviderReadinessFrame::Review {
                    conversation_id,
                    run_id,
                    state: if explicit == Some(false)
                        || matches!(readiness, PrivateProviderReadiness::InvalidInstallation)
                    {
                        ProviderReadinessReviewState::InvalidInstallation
                    } else {
                        ProviderReadinessReviewState::MissingInstall
                    },
                    review: self.review(provider)?,
                }
            } else {
                ProviderReadinessFrame::Unavailable {
                    conversation_id,
                    run_id,
                    reason: ProviderReadinessUnavailable::ProvenanceUnreadable,
                }
            },
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AllowStandalonePublicProviders;

impl StandalonePublicProviderReadiness for AllowStandalonePublicProviders {
    fn is_ready(&self, _: AgentChatProvider) -> Result<bool, String> {
        Ok(true)
    }
}

#[cfg(test)]
#[path = "standalone_provider_readiness_tests.rs"]
mod tests;
