use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

use gent_protocol::{DEFAULT_LOCAL_MODEL_ID, model_catalog::ModelCatalogSelection};
use gent_types::AgentChatProvider;
use serde::{Deserialize, Serialize};

const PREFERENCES_FILE: &str = "model-catalog.json";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelCatalogPreferences {
    default_selection: Option<ModelCatalogSelection>,
    declined_downloads: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ModelCatalogDefaults {
    path: PathBuf,
    preferences: Arc<Mutex<ModelCatalogPreferences>>,
}

impl ModelCatalogDefaults {
    pub(crate) fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join(PREFERENCES_FILE);
        let preferences = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            preferences: Arc::new(Mutex::new(preferences)),
        }
    }

    pub(crate) fn built_in() -> ModelCatalogSelection {
        ModelCatalogSelection {
            provider: AgentChatProvider::Claurst,
            model: DEFAULT_LOCAL_MODEL_ID.into(),
        }
    }

    pub(crate) fn selection(&self) -> ModelCatalogSelection {
        self.lock()
            .default_selection
            .clone()
            .unwrap_or_else(Self::built_in)
    }

    pub(crate) fn save_selection(&self, selection: ModelCatalogSelection) -> Result<(), String> {
        self.update(|preferences| preferences.default_selection = Some(selection))
    }

    pub(crate) fn declined(&self, model_id: &str) -> bool {
        self.lock()
            .declined_downloads
            .iter()
            .any(|declined| declined == model_id)
    }

    pub(crate) fn decline(&self, model_id: &str) -> Result<(), String> {
        if self.declined(model_id) {
            return Ok(());
        }
        self.update(|preferences| preferences.declined_downloads.push(model_id.into()))
    }

    pub(crate) fn accept(&self, model_id: &str) -> Result<(), String> {
        if !self.declined(model_id) {
            return Ok(());
        }
        self.update(|preferences| {
            preferences
                .declined_downloads
                .retain(|declined| declined != model_id);
        })
    }

    fn update(&self, change: impl FnOnce(&mut ModelCatalogPreferences)) -> Result<(), String> {
        let mut preferences = self.lock();
        let mut next = preferences.clone();
        change(&mut next);
        let encoded = serde_json::to_vec(&next).map_err(|error| error.to_string())?;
        let staged = self.path.with_extension("json.tmp");
        fs::write(&staged, encoded)
            .and_then(|()| fs::rename(&staged, &self.path))
            .map_err(|error| format!("could not save model catalog preferences: {error}"))?;
        *preferences = next;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ModelCatalogPreferences> {
        self.preferences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
