use serde_json::{Value, json};

use super::phase::CodexSessionPhase;
use super::wire::{encode, nested_id};
use super::{CodexAppServerSession, CodexSessionError, CodexSessionIngress};

pub(super) fn is_compaction(prompt: &str, attachments: &[Value]) -> bool {
    attachments.is_empty() && gent_types::slash_command(prompt) == Some(("compact", ""))
}

impl CodexAppServerSession {
    pub(super) fn start_compaction(&mut self) -> Result<Vec<u8>, CodexSessionError> {
        let (thread_id, turn_options) = match &self.phase {
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id: None,
                turn_options,
            } => (thread_id.clone(), turn_options.clone()),
            CodexSessionPhase::Failed => return Err(CodexSessionError::SessionFailed),
            CodexSessionPhase::Ready { .. }
            | CodexSessionPhase::AwaitTurn { .. }
            | CodexSessionPhase::AwaitCompaction { .. }
            | CodexSessionPhase::AwaitInjection { .. } => {
                return Err(CodexSessionError::TurnAlreadyActive);
            }
            _ => return Err(CodexSessionError::ThreadNotReady),
        };
        let request_id = self.take_request_id()?;
        let frame = encode(&json!({
            "id": request_id,
            "method": "thread/compact/start",
            "params": {"threadId": thread_id}
        }))?;
        self.phase = CodexSessionPhase::AwaitCompaction {
            request_id: Some(request_id),
            thread_id,
            announced_turn_id: None,
            turn_options,
        };
        Ok(frame)
    }
}

pub(super) fn settled(
    phase: CodexSessionPhase,
    frame: &Value,
) -> (CodexSessionPhase, CodexSessionIngress) {
    let CodexSessionPhase::AwaitCompaction {
        thread_id,
        announced_turn_id,
        turn_options,
        ..
    } = phase
    else {
        return (phase, CodexSessionIngress::Ignored);
    };
    if frame.get("result").is_none() {
        return (CodexSessionPhase::Failed, CodexSessionIngress::Ignored);
    }
    match announced_turn_id {
        Some(turn_id) => (
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: Some(turn_id),
                interrupt_request_id: None,
                turn_options,
            },
            CodexSessionIngress::TurnStarted,
        ),
        None => (
            CodexSessionPhase::AwaitCompaction {
                request_id: None,
                thread_id,
                announced_turn_id: None,
                turn_options,
            },
            CodexSessionIngress::Ignored,
        ),
    }
}

pub(super) fn announced(
    phase: &mut CodexSessionPhase,
    params: &Value,
) -> Result<CodexSessionIngress, CodexSessionError> {
    let CodexSessionPhase::AwaitCompaction {
        request_id,
        thread_id,
        turn_options,
        ..
    } = phase.clone()
    else {
        return Ok(CodexSessionIngress::Ignored);
    };
    if params.get("threadId").and_then(Value::as_str) != Some(thread_id.as_str()) {
        return Ok(CodexSessionIngress::Ignored);
    }
    let turn_id = nested_id(params, "turn")?;
    *phase = match request_id {
        Some(request_id) => CodexSessionPhase::AwaitCompaction {
            request_id: Some(request_id),
            thread_id,
            announced_turn_id: Some(turn_id),
            turn_options,
        },
        None => CodexSessionPhase::Ready {
            thread_id,
            turn_id: Some(turn_id),
            interrupt_request_id: None,
            turn_options,
        },
    };
    Ok(CodexSessionIngress::Ignored)
}
