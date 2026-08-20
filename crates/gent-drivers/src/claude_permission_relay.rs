//! Private Claude permission suggestions retained only at the process boundary.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::claude_control::{
    ClaudePermissionBehavior, ClaudePermissionRequest, encode_permission_response,
    parse_permission_request,
};

/// One active Claude permission relay. Suggestions are never returned from this type.
#[derive(Debug, Default)]
pub(crate) struct ClaudePermissionRelay {
    pending: BTreeMap<String, Vec<Value>>,
}

impl ClaudePermissionRelay {
    /// Retains the provider-native suggestions for a newly received request.
    pub(crate) fn accept(
        &mut self,
        frame: &Value,
    ) -> Result<ClaudePermissionRequest, &'static str> {
        let (request, suggestions) = parse_permission_request(frame)?;
        if self.pending.contains_key(&request.request_id) {
            return Err("duplicateClaudePermissionRequest");
        }
        self.pending.insert(request.request_id.clone(), suggestions);
        Ok(request)
    }

    /// Encodes a decision while keeping provider-native suggestions private.
    pub(crate) fn response(
        &self,
        request_id: &str,
        behavior: ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Option<Vec<u8>> {
        let suggestions = self.pending.get(request_id)?;
        Some(encode_permission_response(
            request_id,
            behavior,
            persist_suggestions,
            suggestions,
        ))
    }

    /// Drops private suggestions only after the response reached the owned process.
    pub(crate) fn settle(&mut self, request_id: &str) {
        self.pending.remove(request_id);
    }
}
