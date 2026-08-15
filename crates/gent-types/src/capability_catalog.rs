//! Durable snapshot of capabilities proven available by a host.

use serde::{Deserialize, Serialize};

use crate::CapabilitySet;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCatalogRecord {
    pub schema_version: u32,
    pub capabilities: CapabilitySet,
}
