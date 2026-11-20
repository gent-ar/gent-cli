//! Documented cooperative Codex turn interruption.

use std::collections::BTreeSet;

use serde_json::json;

use super::wire::encode;
use super::{CodexSessionError, CodexSessionPhase};

pub(super) fn request(
    phase: &mut CodexSessionPhase,
    next_request_id: &mut u64,
    terminal_cleans: &mut BTreeSet<u64>,
) -> Result<Vec<u8>, CodexSessionError> {
    let CodexSessionPhase::Ready {
        thread_id,
        turn_id,
        interrupt_request_id,
        ..
    } = phase
    else {
        return Err(CodexSessionError::TurnNotActive);
    };
    if interrupt_request_id.is_some() {
        return Err(CodexSessionError::InterruptAlreadyRequested);
    }
    let mut take = || {
        let request_id = *next_request_id;
        *next_request_id = next_request_id
            .checked_add(1)
            .ok_or(CodexSessionError::RequestIdExhausted)?;
        Ok(request_id)
    };
    let mut encoded = Vec::new();
    if let Some(turn_id) = turn_id {
        let request_id = take()?;
        *interrupt_request_id = Some(request_id);
        encoded.extend(encode(&json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "turn/interrupt",
            "params": {"threadId": thread_id, "turnId": turn_id},
        }))?);
    }
    let clean_id = take()?;
    terminal_cleans.insert(clean_id);
    encoded.extend(encode(&json!({
        "jsonrpc": "2.0",
        "id": clean_id,
        "method": "thread/backgroundTerminals/clean",
        "params": {"threadId": thread_id},
    }))?);
    Ok(encoded)
}
