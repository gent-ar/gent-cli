use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

use gent_protocol::model_catalog::CatalogModel;
use gent_types::AgentChatProvider;
use serde::{Deserialize, Serialize};

pub(crate) const LAST_KNOWN_FILE: &str = "model-catalog-last-known.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct Listings {
    claude: Vec<CatalogModel>,
    codex: Vec<CatalogModel>,
}

impl Listings {
    fn slot(&mut self, provider: AgentChatProvider) -> Option<&mut Vec<CatalogModel>> {
        match provider {
            AgentChatProvider::Claude => Some(&mut self.claude),
            AgentChatProvider::Codex => Some(&mut self.codex),
            AgentChatProvider::Claurst => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LastKnownModels {
    path: PathBuf,
    listings: Arc<Mutex<Listings>>,
}

impl LastKnownModels {
    pub(crate) fn open(path: PathBuf) -> Self {
        let listings = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            listings: Arc::new(Mutex::new(listings)),
        }
    }

    pub(crate) fn get(&self, provider: AgentChatProvider) -> Option<Vec<CatalogModel>> {
        self.lock()
            .slot(provider)
            .filter(|models| !models.is_empty())
            .cloned()
    }

    pub(crate) fn remember(&self, provider: AgentChatProvider, models: &[CatalogModel]) {
        let mut listings = self.lock();
        let Some(slot) = listings.slot(provider) else {
            return;
        };
        if slot.as_slice() == models {
            return;
        }
        models.clone_into(slot);
        let staged = self.path.with_extension("json.tmp");
        if let Ok(encoded) = serde_json::to_vec(&*listings) {
            let _ = fs::write(&staged, encoded).and_then(|()| fs::rename(&staged, &self.path));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Listings> {
        self.listings.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
