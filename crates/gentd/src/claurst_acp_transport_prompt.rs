use std::collections::{BTreeSet, VecDeque};

use super::{
    ClaurstAcpDrain, ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport,
    ClaurstAcpTransportError, PendingPermission,
};
use gent_types::ToolPhase;
use serde_json::{Value, json};

impl<S: ClaurstAcpStdio> ClaurstAcpTransport<S> {
    pub(crate) fn prompt(
        &mut self,
        session_id: &str,
        prompt: &str,
    ) -> Result<(), ClaurstAcpTransportError> {
        if prompt.trim().is_empty() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        self.prompt_content(session_id, vec![json!({"type": "text", "text": prompt})])
    }

    pub(crate) fn cancel(&mut self, session_id: &str) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized || session_id.is_empty() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        self.write(json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {"sessionId": session_id},
        }))?;
        self.settle_permissions(|_| false).map(drop)
    }

    pub(super) fn settle_permissions(
        &mut self,
        stays: impl Fn(&PendingPermission) -> bool,
    ) -> Result<BTreeSet<String>, ClaurstAcpTransportError> {
        let open: Vec<_> = self
            .pending_permission
            .take()
            .map(|pending| (pending, None))
            .into_iter()
            .chain(
                std::mem::take(&mut self.held_permissions)
                    .into_iter()
                    .map(|(pending, request)| (pending, Some(request))),
            )
            .collect();
        let mut kept = VecDeque::new();
        let mut settled = BTreeSet::new();
        for (pending, request) in open {
            if stays(&pending) {
                kept.push_back((pending, request));
                continue;
            }
            self.write(json!({
                "jsonrpc": "2.0",
                "id": pending.json_rpc_id,
                "result": {"outcome": {"outcome": "cancelled"}},
            }))?;
            settled.insert(pending.tool_use_id);
        }
        self.queued_permissions.retain(|request| {
            kept.iter()
                .any(|(pending, _)| pending.request_id == request.request_id)
        });
        if let Some((pending, request)) = kept.pop_front() {
            self.queued_permissions.extend(request);
            self.pending_permission = Some(pending);
        }
        self.held_permissions = kept
            .into_iter()
            .filter_map(|(pending, request)| request.map(|request| (pending, request)))
            .collect();
        Ok(settled)
    }

    pub(crate) fn drain(
        &mut self,
        limit: u16,
    ) -> Result<ClaurstAcpDrain, ClaurstAcpTransportError> {
        let limit = usize::from(limit.clamp(1, 64));
        let mut drain = ClaurstAcpDrain::default();
        let mut reader_closed = false;
        while drain.facts.len() < limit && drain.permissions.is_empty() {
            if let Some(fact) = self.queued.pop_front() {
                drain.facts.push(fact);
                continue;
            }
            if let Some(permission) = self.queued_permissions.pop_front() {
                drain.permissions.push(permission);
                continue;
            }
            if let Some(terminal) = self.pending_terminal.take() {
                drain.terminal = Some(terminal);
                break;
            }
            let frame = if self.pending_prompt_id.is_none() && self.has_background_commands() {
                match self.background_frame() {
                    Some(frame) => frame,
                    None if self.has_background_commands() => break,
                    None => {
                        reader_closed = true;
                        continue;
                    }
                }
            } else if reader_closed {
                break;
            } else {
                match self.read_frame()? {
                    Some(frame) => frame,
                    None if self.pending_prompt_id.is_none() => break,
                    None => match self.stdio.exited() {
                        Ok(None) => break,
                        Ok(Some(reason)) | Err(reason) => self.read_frame()?.ok_or_else(|| {
                            ClaurstAcpTransportError::Io(format!(
                                "Claurst ended before its turn finished: {reason}"
                            ))
                        })?,
                    },
                }
            };
            if let Some(terminal) = self.handle_frame(frame)? {
                self.pending_terminal = Some(terminal);
            }
        }
        Ok(drain)
    }

    fn background_frame(&mut self) -> Option<Vec<u8>> {
        let frame = match self.read_frame() {
            Ok(None) if matches!(self.stdio.exited(), Ok(None)) => return None,
            Ok(None) => self.read_frame().ok().flatten(),
            Ok(Some(frame)) => Some(frame),
            Err(_) => None,
        };
        if frame.is_none() {
            self.end_background(ToolPhase::Failed);
        }
        frame
    }
}

pub(super) fn prompt_terminal(value: &Value, assistant_output: &str) -> ClaurstAcpTerminal {
    match value.pointer("/result/stopReason").and_then(Value::as_str) {
        Some("end_turn")
            if !assistant_output.is_empty()
                && super::io::unanswered_stop_reason(assistant_output).is_none() =>
        {
            ClaurstAcpTerminal::Completed
        }
        Some("cancelled") => ClaurstAcpTerminal::Interrupted,
        _ => ClaurstAcpTerminal::Failed,
    }
}

pub(super) fn prompt_failure(
    value: &Value,
    assistant_output: &str,
) -> (gent_types::ProviderFailureClassification, &'static str) {
    use gent_types::ProviderFailureClassification::{OutputLimit, Provider};
    let stop_reason = super::io::unanswered_stop_reason(assistant_output)
        .or_else(|| value.pointer("/result/stopReason").and_then(Value::as_str));
    match stop_reason {
        Some("end_turn") => (
            Provider,
            "Claurst ended the turn without an assistant response.",
        ),
        Some("max_tokens") => (
            OutputLimit,
            "Claurst exhausted its output limit before completing the turn.",
        ),
        Some("max_turn_requests") => (
            OutputLimit,
            "Claurst exhausted its turn-request limit before completing the turn.",
        ),
        Some("refusal") => (
            Provider,
            "Claurst refused the request without completing the turn.",
        ),
        _ => (
            Provider,
            "Claurst stopped without a successful end-turn response.",
        ),
    }
}
