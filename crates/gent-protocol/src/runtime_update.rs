//! Additive, read-only runtime-update check frames.
//!
//! This endpoint deliberately has no apply, stage, download, or activation
//! frame. A future externally supervised updater requires a separately
//! versioned and reviewed contract.

use gent_types::{RuntimeUpdateCheckReport, RuntimeUpdateCheckRequest};
use serde::{Deserialize, Serialize};

/// Negotiated capability for a read-only runtime-update check.
pub const RUNTIME_UPDATE_CHECK_CAPABILITY: &str = "runtime-update-check-v1";

/// Frames carried only by the dedicated read-only update-check endpoint.
///
/// Keeping these frames outside [`crate::WireFrame`] avoids changing existing
/// command, receipt, and event semantics while no update authority is exposed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum RuntimeUpdateCheckFrame {
    Request(RuntimeUpdateCheckRequest),
    Report(RuntimeUpdateCheckReport),
}
