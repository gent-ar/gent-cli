//! Private native-identity state for one Codex app-server connection.

use super::{CodexSessionConfig, CodexTurnOptions};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CodexSessionPhase {
    AwaitInitialize {
        request_id: u64,
        config: CodexSessionConfig,
    },
    AwaitThread {
        request_id: u64,
        resumed_thread_id: Option<String>,
        turn_options: CodexTurnOptions,
    },
    ConfirmResumeUnavailable {
        request_id: u64,
    },
    Ready {
        thread_id: String,
        turn_id: Option<String>,
        interrupt_request_id: Option<u64>,
        turn_options: CodexTurnOptions,
    },
    AwaitTurn {
        request_id: u64,
        thread_id: String,
        announced_turn_id: Option<String>,
        turn_options: CodexTurnOptions,
    },
    AwaitCompaction {
        request_id: Option<u64>,
        thread_id: String,
        announced_turn_id: Option<String>,
        turn_options: CodexTurnOptions,
    },
    AwaitInjection {
        request_id: u64,
        thread_id: String,
        turn_options: CodexTurnOptions,
        prompt: String,
        attachments: Vec<serde_json::Value>,
        interrupted_reply: String,
    },
    Failed,
}

pub(super) fn matches_response(phase: &CodexSessionPhase, response_id: u64) -> bool {
    match phase {
        CodexSessionPhase::AwaitInitialize { request_id, .. }
        | CodexSessionPhase::AwaitThread { request_id, .. }
        | CodexSessionPhase::ConfirmResumeUnavailable { request_id }
        | CodexSessionPhase::AwaitTurn { request_id, .. }
        | CodexSessionPhase::AwaitInjection { request_id, .. } => *request_id == response_id,
        CodexSessionPhase::AwaitCompaction { request_id, .. } => *request_id == Some(response_id),
        CodexSessionPhase::Ready {
            interrupt_request_id,
            ..
        } => interrupt_request_id == &Some(response_id),
        CodexSessionPhase::Failed => false,
    }
}
