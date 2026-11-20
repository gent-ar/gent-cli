use serde_json::Value;

use super::{phase::CodexSessionPhase, types::CodexSessionIngress, wire::nested_id};

pub(super) fn reduce(
    phase: &mut CodexSessionPhase,
    method: &str,
    params: Option<&Value>,
) -> Result<CodexSessionIngress, super::types::CodexSessionError> {
    let Some(params) = params else {
        return Ok(CodexSessionIngress::Ignored);
    };
    if method == "turn/started" && matches!(phase, CodexSessionPhase::AwaitCompaction { .. }) {
        return super::compaction::announced(phase, params);
    }
    let current = phase.clone();
    match (method, current) {
        (
            "turn/started",
            CodexSessionPhase::AwaitTurn {
                request_id,
                thread_id,
                turn_options,
                ..
            },
        ) if params.get("threadId").and_then(Value::as_str) == Some(thread_id.as_str()) => {
            let turn_id = nested_id(params, "turn")?;
            *phase = CodexSessionPhase::AwaitTurn {
                request_id,
                thread_id,
                announced_turn_id: Some(turn_id),
                turn_options,
            };
            Ok(CodexSessionIngress::Ignored)
        }
        (
            "turn/started",
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id,
                turn_options,
            },
        ) if params.get("threadId").and_then(Value::as_str) == Some(thread_id.as_str()) => {
            let turn_id = nested_id(params, "turn")?;
            *phase = CodexSessionPhase::Ready {
                thread_id,
                turn_id: Some(turn_id),
                interrupt_request_id,
                turn_options,
            };
            Ok(CodexSessionIngress::ProviderTurnStarted)
        }
        (
            "turn/completed",
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: Some(turn_id),
                interrupt_request_id,
                turn_options,
            },
        ) if params.get("threadId").and_then(Value::as_str) == Some(thread_id.as_str())
            && nested_id(params, "turn")? == turn_id =>
        {
            *phase = CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id,
                turn_options,
            };
            Ok(CodexSessionIngress::TurnEnded)
        }
        _ => Ok(CodexSessionIngress::Ignored),
    }
}
