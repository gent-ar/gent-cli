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
        (
            "turn/completed",
            CodexSessionPhase::AwaitTurn {
                thread_id,
                announced_turn_id: Some(_),
                turn_options,
                ..
            },
        ) if params.as_object().is_some_and(|params| params.is_empty()) => {
            *phase = CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id: None,
                turn_options,
            };
            Ok(CodexSessionIngress::TurnEnded)
        }
        (
            "turn/completed",
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: Some(_),
                interrupt_request_id,
                turn_options,
            },
        ) if params.as_object().is_some_and(|params| params.is_empty()) => {
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
