//! Content-safe, read-only values supplied by a future legacy-host observer tap.

use serde::{Deserialize, Serialize};

use crate::{ConversationLiveStatus, NormalizedLifecycleSignal, ReceiptId};

/// One normalized lifecycle observation from the legacy host.
///
/// It intentionally excludes transcript content, provider frames, sessions, paths, endpoints,
/// credentials, and any mutation capability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyLifecycleTap {
    pub cursor: u64,
    pub event_id: String,
    pub receipt_id: ReceiptId,
    pub signal: NormalizedLifecycleSignal,
    pub reported: ConversationLiveStatus,
}

/// Stable classifications for a read-only projection comparison.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObserverDiagnosticCode {
    Duplicate,
    CursorGap,
    StatusMismatch,
}

/// Content-safe diagnostic for a future comparison artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserverDiagnostic {
    pub code: ObserverDiagnosticCode,
    pub cursor: u64,
    pub event_id: String,
    pub receipt_id: ReceiptId,
    pub expected: Option<ConversationLiveStatus>,
    pub reported: Option<ConversationLiveStatus>,
}
