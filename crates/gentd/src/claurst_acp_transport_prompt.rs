use super::{
    ClaurstAcpDrain, ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport,
    ClaurstAcpTransportError,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const CANCEL_SETTLE: Duration = Duration::from_secs(5);

impl<S: ClaurstAcpStdio> ClaurstAcpTransport<S> {
    pub(crate) fn prompt(
        &mut self,
        session_id: &str,
        prompt: &str,
    ) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized {
            return Err(ClaurstAcpTransportError::Uninitialized);
        }
        if session_id.is_empty() || prompt.trim().is_empty() || self.pending_prompt_id.is_some() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        self.prompt_content(session_id, vec![json!({"type": "text", "text": prompt})])
    }

    fn cancel_expired(&self) -> bool {
        self.pending_prompt_id.is_some()
            && self
                .cancel_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
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
        self.cancel_deadline = Some(Instant::now() + CANCEL_SETTLE);
        self.queued_permissions.clear();
        let held = std::mem::take(&mut self.held_permissions)
            .into_iter()
            .map(|(pending, _)| pending);
        for pending in self.pending_permission.take().into_iter().chain(held) {
            self.write(json!({
                "jsonrpc": "2.0",
                "id": pending.json_rpc_id,
                "result": {"outcome": {"outcome": "cancelled"}},
            }))?;
        }
        Ok(())
    }

    pub(crate) fn drain(
        &mut self,
        limit: u16,
    ) -> Result<ClaurstAcpDrain, ClaurstAcpTransportError> {
        let limit = usize::from(limit.clamp(1, 64));
        let mut drain = ClaurstAcpDrain::default();
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
            let frame = match self.read_frame()? {
                Some(frame) => frame,
                None if self.cancel_expired() => {
                    self.pending_prompt_id = None;
                    self.pending_terminal = Some(ClaurstAcpTerminal::Interrupted);
                    continue;
                }
                None if self.pending_prompt_id.is_none() => break,
                None => match self.stdio.exited() {
                    Ok(None) => break,
                    Ok(Some(reason)) | Err(reason) => self.read_frame()?.ok_or_else(|| {
                        ClaurstAcpTransportError::Io(format!(
                            "Claurst ended before its turn finished: {reason}"
                        ))
                    })?,
                },
            };
            if let Some(terminal) = self.handle_frame(frame)? {
                self.pending_terminal = Some(terminal);
            }
        }
        Ok(drain)
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
    use gent_types::ProviderFailureClassification::{ContextLimit, OutputLimit, Provider};
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
            ContextLimit,
            "Claurst refused the request without completing the turn.",
        ),
        _ => (
            Provider,
            "Claurst stopped without a successful end-turn response.",
        ),
    }
}
