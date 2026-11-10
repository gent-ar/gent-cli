use std::time::{Duration, Instant};

use gent_protocol::{
    LocalModelInstallState,
    model_catalog::{ModelListing, ProviderAvailability, ProviderModelCatalog},
};
use gent_types::{AgentChatEffort, AgentChatProvider, AgentChatRejection, AgentChatSelection};

use super::ModelCatalogService;
use crate::agent_chat_intent_error::AgentChatIntentError;

const RELOAD_WAIT: Duration = Duration::from_secs(25);
const FIRST_LISTING_WAIT: Duration = Duration::from_secs(3);

impl ModelCatalogService {
    pub(crate) fn fit(
        &self,
        selection: &AgentChatSelection,
    ) -> Result<AgentChatSelection, AgentChatIntentError> {
        self.reload_uncached(selection.provider);
        let Some(listing) = self.first_listing(selection.provider) else {
            return Ok(selection.clone());
        };
        if listing.listing != ModelListing::Ready {
            return Ok(AgentChatSelection {
                effort: AgentChatEffort::Medium,
                ..selection.clone()
            });
        }
        if listing.availability != ProviderAvailability::Ready {
            return Ok(selection.clone());
        }
        let Some(model) = listing
            .models
            .iter()
            .find(|model| model.id == selection.model)
        else {
            if accepts_unlisted_models(selection.provider) {
                return Ok(selection.clone());
            }
            let models = listing
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>();
            return Err(rejected(
                AgentChatRejection::SelectionModelUnavailable,
                &format!("{} does not offer `{}`", listing.label, selection.model),
                &models,
            ));
        };
        if model
            .local
            .as_ref()
            .is_some_and(|local| local.install == LocalModelInstallState::Missing)
        {
            return Err(rejected(
                AgentChatRejection::SelectionModelUnavailable,
                &format!("the file for `{}` is missing or changed", model.label),
                &listing
                    .models
                    .iter()
                    .filter(|other| other.id != model.id)
                    .map(|other| other.id.as_str())
                    .collect::<Vec<_>>(),
            ));
        }
        Ok(AgentChatSelection {
            effort: selection.effort.nearest_in(&model.efforts),
            ..selection.clone()
        })
    }

    fn first_listing(&self, provider: AgentChatProvider) -> Option<ProviderModelCatalog> {
        let deadline = Instant::now() + FIRST_LISTING_WAIT;
        loop {
            let entry = self
                .read(false)
                .providers
                .into_iter()
                .find(|entry| entry.provider == provider)?;
            if entry.listing != ModelListing::Loading || Instant::now() >= deadline {
                return Some(entry);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub(crate) fn reload_uncached(&self, provider: AgentChatProvider) {
        let Some(index) = self
            .sources
            .iter()
            .position(|source| source.provider() == provider && source.ttl().is_zero())
        else {
            return;
        };
        let deadline = Instant::now() + RELOAD_WAIT;
        loop {
            {
                let mut slots = self.slots();
                if !slots[index].loading {
                    slots[index].loading = true;
                    break;
                }
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.load(index);
    }
}

pub(super) const fn accepts_unlisted_models(provider: AgentChatProvider) -> bool {
    matches!(
        provider,
        AgentChatProvider::Claude | AgentChatProvider::Codex
    )
}

fn rejected(
    rejection: AgentChatRejection,
    subject: &str,
    choices: &[&str],
) -> AgentChatIntentError {
    AgentChatIntentError {
        code: rejection.code(),
        message: format!("{subject}; choose one of: {}", choices.join(", ")),
    }
}
