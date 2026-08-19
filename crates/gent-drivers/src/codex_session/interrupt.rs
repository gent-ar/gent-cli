//! Documented cooperative Codex turn interruption.

use serde_json::json;

use super::{CodexSessionError, CodexSessionPhase};

pub(super) fn request(
    phase: &mut CodexSessionPhase,
    next_request_id: &mut u64,
) -> Result<Vec<u8>, CodexSessionError> {
    let CodexSessionPhase::Ready {
        thread_id,
        turn_id: Some(turn_id),
        interrupt_request_id,
        ..
    } = phase
    else {
        return Err(CodexSessionError::TurnNotActive);
    };
    if interrupt_request_id.is_some() {
        return Err(CodexSessionError::InterruptAlreadyRequested);
    }
    let request_id = *next_request_id;
    *next_request_id = next_request_id
        .checked_add(1)
        .ok_or(CodexSessionError::RequestIdExhausted)?;
    *interrupt_request_id = Some(request_id);
    let frame = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "turn/interrupt",
        "params": {"threadId": thread_id, "turnId": turn_id},
    });
    let mut encoded = serde_json::to_vec(&frame).map_err(|_| CodexSessionError::Serialization)?;
    encoded.push(b'\n');
    Ok(encoded)
}
