use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use gent_protocol::model_catalog::{
    CatalogModel, ModelCatalog, ModelCatalogSelection, ModelListing, ProviderAvailability,
    ProviderModelCatalog,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, ProviderAuthLifecycle,
    ProviderAuthProvider,
};

use super::defaults::ModelCatalogDefaults;
use crate::{
    provider_auth_api::{ProviderAuthPort, STATUS_SETTLE_BOUND},
    provider_launch_budget::{ProbeRetry, ProviderLaunchError},
};

pub(crate) const PROVIDER_DEFAULT_MODEL: &str = "default";

pub(crate) struct ProviderListing {
    pub(crate) availability: ProviderAvailability,
    pub(crate) models: Vec<CatalogModel>,
}

pub(crate) trait ModelCatalogSource: Send + Sync {
    fn provider(&self) -> AgentChatProvider;
    fn label(&self) -> &'static str;
    fn ttl(&self) -> Duration;
    fn load(&self) -> Result<ProviderListing, ProviderLaunchError>;
    fn revision(&self) -> Option<String> {
        None
    }
}

struct Slot {
    entry: ProviderModelCatalog,
    loaded_at: Option<Instant>,
    loading: bool,
    revision: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ModelCatalogService {
    sources: Arc<Vec<Arc<dyn ModelCatalogSource>>>,
    slots: Arc<Mutex<Vec<Slot>>>,
    defaults: ModelCatalogDefaults,
    retry: ProbeRetry,
}

impl ModelCatalogService {
    pub(crate) fn new(
        sources: Vec<Arc<dyn ModelCatalogSource>>,
        defaults: ModelCatalogDefaults,
    ) -> Self {
        let slots = sources
            .iter()
            .map(|source| Slot {
                entry: ProviderModelCatalog {
                    provider: source.provider(),
                    label: source.label().into(),
                    availability: ProviderAvailability::Checking,
                    listing: ModelListing::Loading,
                    models: Vec::new(),
                },
                loaded_at: None,
                loading: false,
                revision: None,
            })
            .collect();
        Self {
            sources: Arc::new(sources),
            slots: Arc::new(Mutex::new(slots)),
            defaults,
            retry: ProbeRetry::BACKGROUND,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_retry(mut self, retry: ProbeRetry) -> Self {
        self.retry = retry;
        self
    }

    pub(crate) fn read(&self, refresh: bool) -> ModelCatalog {
        let due = self.claim_due(refresh);
        for index in due {
            let service = self.clone();
            std::thread::spawn(move || service.load(index));
        }
        self.snapshot()
    }

    pub(crate) fn snapshot(&self) -> ModelCatalog {
        ModelCatalog {
            default_selection: self.defaults.selection(),
            providers: self.slots().iter().map(|slot| slot.entry.clone()).collect(),
        }
    }

    pub(crate) fn set_default(
        &self,
        selection: ModelCatalogSelection,
    ) -> Result<ModelCatalog, String> {
        if self.listed_without(&selection) {
            return Err("the default model is not in Gent's model catalog".into());
        }
        self.defaults.save_selection(selection)?;
        Ok(self.snapshot())
    }

    pub(crate) fn default_agent_selection(&self) -> AgentChatSelection {
        let selection = self.defaults.selection();
        let effort = self
            .model(&selection)
            .and_then(|model| model.default_effort)
            .unwrap_or(AgentChatEffort::Medium);
        AgentChatSelection {
            provider: selection.provider,
            model: selection.model,
            effort,
            mode: AgentChatMode::Agent,
        }
    }

    fn listed_without(&self, selection: &ModelCatalogSelection) -> bool {
        self.slots().iter().any(|slot| {
            slot.entry.provider == selection.provider
                && slot.entry.listing == ModelListing::Ready
                && !slot
                    .entry
                    .models
                    .iter()
                    .any(|model| model.id == selection.model)
        })
    }

    fn model(&self, selection: &ModelCatalogSelection) -> Option<CatalogModel> {
        self.slots()
            .iter()
            .filter(|slot| slot.entry.provider == selection.provider)
            .flat_map(|slot| slot.entry.models.iter())
            .find(|model| model.id == selection.model)
            .cloned()
    }

    fn claim_due(&self, refresh: bool) -> Vec<usize> {
        let mut slots = self.slots();
        let mut due = Vec::new();
        for (index, slot) in slots.iter_mut().enumerate() {
            let revision = self.sources[index].revision();
            let changed = slot.loaded_at.is_some() && slot.revision != revision;
            let stale = slot
                .loaded_at
                .is_none_or(|loaded| loaded.elapsed() >= self.sources[index].ttl());
            if slot.loading || !(refresh || changed || stale) {
                continue;
            }
            slot.loading = true;
            slot.revision = revision;
            if refresh || changed || slot.loaded_at.is_none() {
                slot.entry.listing = ModelListing::Loading;
            }
            due.push(index);
        }
        due
    }

    fn load(&self, index: usize) {
        let mut attempt = 1;
        let result = loop {
            match self.sources[index].load() {
                Err(ProviderLaunchError::TimedOut(_)) if attempt < self.retry.attempts => {
                    attempt += 1;
                    std::thread::sleep(self.retry.delay);
                }
                result => break result,
            }
        };
        let mut slots = self.slots();
        let slot = &mut slots[index];
        slot.loading = false;
        slot.loaded_at =
            (!matches!(result, Err(ProviderLaunchError::TimedOut(_)))).then(Instant::now);
        match result {
            Ok(listing) => {
                slot.entry.availability = listing.availability;
                slot.entry.models = listing.models;
                slot.entry.listing = ModelListing::Ready;
            }
            Err(error) => {
                slot.entry.listing = ModelListing::Failed {
                    message: error.to_string(),
                };
            }
        }
    }

    fn slots(&self) -> MutexGuard<'_, Vec<Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn effort(value: &str) -> Option<AgentChatEffort> {
    match value {
        "low" => Some(AgentChatEffort::Low),
        "medium" => Some(AgentChatEffort::Medium),
        "high" => Some(AgentChatEffort::High),
        "xhigh" => Some(AgentChatEffort::XHigh),
        "max" => Some(AgentChatEffort::Max),
        "ultra" => Some(AgentChatEffort::Ultra),
        _ => None,
    }
}

pub(crate) fn provider_default_model() -> CatalogModel {
    CatalogModel {
        id: PROVIDER_DEFAULT_MODEL.into(),
        label: "Default".into(),
        description: None,
        is_default: true,
        efforts: Vec::new(),
        default_effort: None,
        local: None,
    }
}

pub(crate) fn public_availability(
    auth: &dyn ProviderAuthPort,
    provider: ProviderAuthProvider,
) -> Result<ProviderAvailability, ProviderLaunchError> {
    let deadline = Instant::now() + STATUS_SETTLE_BOUND;
    let mut lifecycle = auth.check_status(provider).lifecycle;
    while lifecycle == ProviderAuthLifecycle::Checking && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        lifecycle = auth.check_status(provider).lifecycle;
    }
    match lifecycle {
        ProviderAuthLifecycle::Checking => Err(ProviderLaunchError::TimedOut(
            "the provider account is still being checked".into(),
        )),
        ProviderAuthLifecycle::NotInstalled => Ok(ProviderAvailability::NotInstalled),
        ProviderAuthLifecycle::Authenticated => Ok(ProviderAvailability::Ready),
        ProviderAuthLifecycle::Failed | ProviderAuthLifecycle::ProviderChanged => Err(
            ProviderLaunchError::Failed("the provider executable could not be verified".into()),
        ),
        ProviderAuthLifecycle::Unauthenticated
        | ProviderAuthLifecycle::ChallengeOffered
        | ProviderAuthLifecycle::Verifying
        | ProviderAuthLifecycle::Expired
        | ProviderAuthLifecycle::Cancelled
        | ProviderAuthLifecycle::TimedOut => Ok(ProviderAvailability::SignedOut),
    }
}

#[path = "model_catalog_selection.rs"]
mod selection;

#[cfg(test)]
#[path = "model_catalog_service_tests.rs"]
mod tests;
