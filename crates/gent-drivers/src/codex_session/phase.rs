//! Private native-identity state for one Codex app-server connection.

use super::CodexSessionConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CodexSessionPhase {
    AwaitInitialize {
        request_id: u64,
        config: CodexSessionConfig,
    },
    AwaitThread {
        request_id: u64,
        resumed_thread_id: Option<String>,
    },
    Ready {
        thread_id: String,
        turn_id: Option<String>,
    },
    AwaitTurn {
        request_id: u64,
        thread_id: String,
        announced_turn_id: Option<String>,
    },
    Failed,
}

pub(super) fn matches_response(phase: &CodexSessionPhase, response_id: u64) -> bool {
    match phase {
        CodexSessionPhase::AwaitInitialize { request_id, .. }
        | CodexSessionPhase::AwaitThread { request_id, .. }
        | CodexSessionPhase::AwaitTurn { request_id, .. } => *request_id == response_id,
        CodexSessionPhase::Ready { .. } | CodexSessionPhase::Failed => false,
    }
}
