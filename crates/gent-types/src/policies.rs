//! Versioned, secret-free workspace permission policy records.

use serde::{Deserialize, Serialize};

/// The policy domain covered by a record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyScope {
    ProviderPermissions,
}

/// One immutable revision of a workspace allow-list policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRecord {
    pub policy_id: String,
    pub workspace_id: String,
    pub scope: PolicyScope,
    pub revision: u64,
    /// Canonically sorted provider tool names. Credentials and endpoint data are never policy.
    pub allowed_tools: Vec<String>,
}
