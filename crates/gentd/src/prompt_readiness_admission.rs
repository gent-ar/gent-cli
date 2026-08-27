//! Daemon-owned admission from a held prompt to a verified provider-ready lifecycle wake.

use gent_types::{Command, Event, HostEpoch, ProviderPromptReadinessFailureBinding, ReceiptId};
use sha2::{Digest, Sha256};

pub(crate) fn decision(
    binding: &gent_types::ProviderPromptReadinessBinding,
    host_epoch: HostEpoch,
) -> Result<(Command, Event), String> {
    let payload = serde_json::to_value(binding).map_err(|error| error.to_string())?;
    let identity = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
    ));
    let receipt_id = ReceiptId(format!("daemon-readiness:{identity}"));
    let command = Command {
        receipt_id: receipt_id.clone(),
        idempotency_key: format!("daemon-readiness:{identity}"),
        host_epoch,
        kind: "agentChatProviderReadiness".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: format!("daemon-readiness:{identity}:ready"),
        receipt_id,
        host_epoch,
        kind: "agentChatProviderReady".into(),
        payload,
    };
    Ok((command, terminal))
}

pub(crate) fn failure(
    binding: &ProviderPromptReadinessFailureBinding,
    host_epoch: HostEpoch,
) -> Result<(Command, Event), String> {
    if !binding.is_valid() {
        return Err("provider readiness failure binding is invalid".into());
    }
    let payload = serde_json::to_value(binding).map_err(|error| error.to_string())?;
    let identity = hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
    ));
    let receipt_id = ReceiptId(format!("daemon-readiness-failure:{identity}"));
    let command = Command {
        receipt_id: receipt_id.clone(),
        idempotency_key: format!("daemon-readiness-failure:{identity}"),
        host_epoch,
        kind: "agentChatProviderReadinessFailure".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: format!("daemon-readiness-failure:{identity}:failed"),
        receipt_id,
        host_epoch,
        kind: "agentChatProviderReadinessFailed".into(),
        payload,
    };
    Ok((command, terminal))
}
