//! Private, operation-bound Claude permission data; never a public preview.
use crate::claude_control::{
    ClaudePermissionBehavior, ClaudePermissionRequest, MAX_PRESERVED_INPUT_BYTES,
    encode_permission_response_preserving_input, encode_permission_response_with_input,
    parse_permission_request, valid_explicit_updated_input,
};
use serde_json::Value;
use std::collections::BTreeMap;

// Soft bound for newly retained original input. Existing large-input approvals
// still use the previous omitted-updatedInput encoding; they are not rejected.
const MAX_RETAINED_ORIGINAL_BYTES: usize = 4 * 1024 * 1024;
struct PendingPermission {
    suggestions: Vec<Value>,
    original_input: Option<Value>,
    retained_bytes: usize,
}
#[derive(Default)]
pub(crate) struct ClaudePermissionRelay {
    pending: BTreeMap<String, PendingPermission>,
    retained_bytes: usize,
}
impl std::fmt::Debug for ClaudePermissionRelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudePermissionRelay")
            .field("pending_count", &self.pending.len())
            .field("retained_original_bytes", &self.retained_bytes)
            .finish_non_exhaustive()
    }
}
impl ClaudePermissionRelay {
    pub(crate) fn accept(
        &mut self,
        frame: &Value,
    ) -> Result<ClaudePermissionRequest, &'static str> {
        let (request, suggestions) = parse_permission_request(frame)?;
        if self.pending.contains_key(&request.request_id) {
            return Err("duplicateClaudePermissionRequest");
        }
        let captured = frame
            .get("request")
            .and_then(|r| r.get("input"))
            .filter(|input| input.is_object())
            .and_then(|input| {
                let bytes = serde_json::to_vec(input).ok()?.len();
                (bytes <= MAX_PRESERVED_INPUT_BYTES
                    && self
                        .retained_bytes
                        .checked_add(bytes)
                        .is_some_and(|total| total <= MAX_RETAINED_ORIGINAL_BYTES))
                .then(|| (input.clone(), bytes))
            });
        let (original_input, retained_bytes) =
            captured.map_or((None, 0), |(value, bytes)| (Some(value), bytes));
        self.pending.insert(
            request.request_id.clone(),
            PendingPermission {
                suggestions,
                original_input,
                retained_bytes,
            },
        );
        self.retained_bytes += retained_bytes;
        Ok(request)
    }
    pub(crate) fn response_with_input(
        &self,
        request_id: &str,
        behavior: ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<&Value>,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        let Some(pending) = self.pending.get(request_id) else {
            return Ok(None);
        };
        if behavior == ClaudePermissionBehavior::Deny {
            return Ok(Some(encode_permission_response_with_input(
                request_id,
                behavior,
                false,
                &pending.suggestions,
                None,
            )));
        }
        if let Some(input) = updated_input {
            if !valid_explicit_updated_input(input) {
                return Err(
                    "Claude permission update must be a JSON object of at most16384 bytes; no response was sent and the request remains pending",
                );
            }
            return Ok(Some(encode_permission_response_with_input(
                request_id,
                behavior,
                persist_suggestions,
                &pending.suggestions,
                Some(input),
            )));
        }
        if let Some(original) = &pending.original_input {
            if let Some(bytes) = encode_permission_response_preserving_input(
                request_id,
                behavior,
                persist_suggestions,
                &pending.suggestions,
                original,
            ) {
                return Ok(Some(bytes));
            }
        }
        // Preserve previously working behavior when the input or combined reply is
        // too large to echo safely; never silently substitute another operation's input.
        Ok(Some(encode_permission_response_with_input(
            request_id,
            behavior,
            persist_suggestions,
            &pending.suggestions,
            None,
        )))
    }
    pub(crate) fn settle(&mut self, request_id: &str) {
        if let Some(pending) = self.pending.remove(request_id) {
            self.retained_bytes -= pending.retained_bytes;
        }
    }
    pub(crate) fn cancel(&mut self, frame: &Value) -> bool {
        let id = frame
            .get("request_id")
            .or_else(|| frame.get("requestId"))
            .or_else(|| frame.pointer("/params/request_id"))
            .or_else(|| frame.pointer("/params/requestId"))
            .and_then(Value::as_str);
        let Some(id) = id else { return false };
        if !self.pending.contains_key(id) {
            return false;
        }
        self.settle(id);
        true
    }
}
#[cfg(test)]
#[path = "claude_permission_relay_input_tests.rs"]
mod input_tests;
