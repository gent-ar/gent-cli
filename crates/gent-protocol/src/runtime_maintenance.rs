//! Additive, read-only runtime-maintenance frames.

use gent_types::{RuntimeMaintenanceReport, RuntimeMaintenanceRequest};
use serde::{Deserialize, Serialize};

/// Negotiated capability for durable update-attempt maintenance state.
pub const RUNTIME_MAINTENANCE_CAPABILITY: &str = "runtime-maintenance-v1";

/// Read-only request/response frames separate from runtime update check discovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum RuntimeMaintenanceFrame {
    Request(RuntimeMaintenanceRequest),
    Report(Box<RuntimeMaintenanceReport>),
}
