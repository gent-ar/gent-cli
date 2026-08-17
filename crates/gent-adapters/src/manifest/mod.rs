//! Pure declarative provider manifests and their nested interpretation rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod interpretation;
mod validation;

pub use validation::ManifestError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeclarativeAdapterManifest {
    pub id: String,
    pub protocol_version: u16,
    /// Provider frame type to normalized event kind mapping.
    pub event_map: BTreeMap<String, String>,
}

impl DeclarativeAdapterManifest {
    /// Validates the portable subset before an adapter is registered.
    ///
    /// # Errors
    /// Returns an error for missing identity or unsupported event mappings.
    pub fn validate(&self) -> Result<(), ManifestError> {
        validation::validate(self)
    }
}
