//! Durable terminal settlement snapshot for one provider-bound agent-chat turn.

use serde::{Deserialize, Serialize};

use crate::{DurableTurnPhase, HostEpoch};

/// Restart-safe terminal state jointly proven by a settled dispatch and terminal turn phase.
///
/// This contains only Gent-owned identities and durable cursor state. It never carries prompt
/// content, a provider-native session, credentials, or raw provider output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatTerminalSettlement {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub host_epoch: HostEpoch,
    pub cursor: u64,
    pub phase: DurableTurnPhase,
}
