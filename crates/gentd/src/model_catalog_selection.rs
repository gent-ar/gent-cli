use std::time::{Duration, Instant};

use gent_protocol::model_catalog::{ModelListing, ProviderAvailability};
use gent_types::{AgentChatProvider, AgentChatRejection, AgentChatSelection};

use super::ModelCatalogService;
use crate::agent_chat_intent_error::AgentChatIntentError;

const FIRST_LISTING_WAIT: Duration = Duration::from_secs(25);

impl ModelCatalogService {
    pub(crate) fn validate(
        &self,
        selection: &AgentChatSelection,
    ) -> Result<(), AgentChatIntentError> {
        self.await_first_listing(selection.provider);
        let slots = self.slots();
        let Some(listing) = slots.iter().map(|slot| &slot.entry).find(|entry| {
            entry.provider == selection.provider
                && entry.listing == ModelListing::Ready
                && entry.availability == ProviderAvailability::Ready
        }) else {
            return Ok(());
        };
        let Some(model) = listing
            .models
            .iter()
            .find(|model| model.id == selection.model)
        else {
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
        if model.efforts.is_empty() || model.efforts.contains(&selection.effort) {
            return Ok(());
        }
        let efforts = model
            .efforts
            .iter()
            .filter_map(|effort| serde_json::to_value(effort).ok())
            .filter_map(|effort| effort.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        Err(rejected(
            AgentChatRejection::SelectionEffortUnavailable,
            &format!("{} does not offer this effort", model.label),
            &efforts.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
    }

    fn await_first_listing(&self, provider: AgentChatProvider) {
        self.read(false);
        let deadline = Instant::now() + FIRST_LISTING_WAIT;
        while Instant::now() < deadline
            && self.slots().iter().any(|slot| {
                slot.entry.provider == provider && slot.loading && slot.loaded_at.is_none()
            })
        {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
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
