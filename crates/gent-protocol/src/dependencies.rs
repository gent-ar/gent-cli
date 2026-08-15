//! Public dependency planning and receipt-bound mutation DTOs.

use std::str::FromStr;

use gent_types::{HostEpoch, Receipt, ReceiptId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ProtocolError;

/// A publicly installable provider. Private bridges are intentionally excluded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyProvider {
    Claude,
    Codex,
}

impl DependencyProvider {
    /// Returns the stable public provider identifier used in durable locks.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

impl FromStr for DependencyProvider {
    type Err = ProtocolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err(ProtocolError::UnsupportedProvider(value.into())),
        }
    }
}

/// An explicit action a user may request for a public provider dependency.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyAction {
    Install,
    Update,
}

impl FromStr for DependencyAction {
    type Err = ProtocolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "install" => Ok(Self::Install),
            "update" => Ok(Self::Update),
            _ => Err(ProtocolError::UnsupportedDependencyAction(value.into())),
        }
    }
}

/// A read-only request for a dependency action plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPlanRequest {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
}

/// An explicit confirmation to act on a prior plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyActionRequest {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
    pub consent_granted: bool,
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    /// Digest of the exact daemon-issued plan this action confirms.
    pub reviewed_plan_digest: String,
}

/// Human-readable, vendor-directed plan. Gent never embeds a provider installer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPlan {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
    pub instruction: String,
    pub consent_required: bool,
    /// Stable digest the client must echo when confirming this plan.
    pub reviewed_plan_digest: String,
}

impl DependencyPlan {
    /// Builds a plan and its review digest from canonical, user-visible fields.
    #[must_use]
    pub fn reviewed(
        provider: DependencyProvider,
        action: DependencyAction,
        instruction: impl Into<String>,
        consent_required: bool,
    ) -> Self {
        let instruction = instruction.into();
        let reviewed_plan_digest =
            dependency_plan_digest(provider, action, &instruction, consent_required);
        Self {
            provider,
            action,
            instruction,
            consent_required,
            reviewed_plan_digest,
        }
    }
}

/// Returns a canonical SHA-256 digest for the visible dependency plan fields.
///
/// # Panics
/// Panics only if the fixed, serializable plan DTO cannot be encoded as JSON.
#[must_use]
pub fn dependency_plan_digest(
    provider: DependencyProvider,
    action: DependencyAction,
    instruction: &str,
    consent_required: bool,
) -> String {
    let document = serde_json::json!({
        "action": action,
        "consentRequired": consent_required,
        "instruction": instruction,
        "provider": provider,
    });
    let bytes = serde_json::to_vec(&document).expect("dependency plan fields serialize");
    hex::encode(Sha256::digest(bytes))
}

/// Result of evaluating an explicit dependency action request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyActionState {
    ConsentRequired,
    Completed,
    Failed,
    PlanMismatch,
    Unprovable,
}

/// Terminal result of an explicit dependency action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyActionResult {
    pub plan: DependencyPlan,
    pub state: DependencyActionState,
    pub receipt: Receipt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
