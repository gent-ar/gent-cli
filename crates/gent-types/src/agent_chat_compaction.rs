//! Provider-neutral, normalized compaction facts for daemon-owned recovery.

use serde::{Deserialize, Serialize};

/// A bounded classification for an already-normalized provider compaction failure.
///
/// This deliberately excludes provider error strings and native session values.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatCompactionFailure {
    /// The provider reported that its own compactor had too few reducible groups.
    TooFewGroups,
    /// The provider failed its compaction attempt for another normalized reason.
    ///
    /// This is durable diagnostic state, not authority to create a recovery child.
    ProviderFailed,
}

/// A normalized provider compaction lifecycle fact.
///
/// Only a daemon-owned adapter may construct these facts. They are not a client command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatCompactionFact {
    Started {
        turn_id: String,
    },
    Completed {
        turn_id: String,
    },
    Failed {
        turn_id: String,
        failure: AgentChatCompactionFailure,
    },
}

#[cfg(test)]
mod tests {
    use super::{AgentChatCompactionFact, AgentChatCompactionFailure};

    #[test]
    fn facts_never_accept_raw_provider_detail() {
        let value = serde_json::json!({
            "type": "failed", "turnId": "turn-1", "failure": "tooFewGroups",
            "providerError": "must not cross the contract"
        });
        assert!(serde_json::from_value::<AgentChatCompactionFact>(value).is_err());
        assert_eq!(
            serde_json::from_value::<AgentChatCompactionFact>(serde_json::json!({
                "type": "failed", "turnId": "turn-1", "failure": "tooFewGroups"
            }))
            .unwrap(),
            AgentChatCompactionFact::Failed {
                turn_id: "turn-1".into(),
                failure: AgentChatCompactionFailure::TooFewGroups,
            }
        );
    }
}
