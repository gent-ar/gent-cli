//! Read-only maintenance facts for compatible local clients.

use crate::{HostEpoch, RuntimeUpdateRecord};
use serde::{Deserialize, Serialize};

/// A durable update attempt requested by a client through the maintenance endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMaintenanceRequest {
    pub attempt_id: String,
}

/// Versioned, content-free state for one durable update attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeMaintenanceReport {
    pub host_epoch: HostEpoch,
    pub ingress_closed: bool,
    pub record: RuntimeUpdateRecord,
}
