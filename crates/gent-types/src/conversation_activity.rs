//! Typed, content-free activity facts for one conversation run.

use serde::{Deserialize, Serialize};

use crate::{HostEpoch, RootActivity, ToolActivity, TurnPhase, WorkPhase};

/// Version of the conversation-activity value contract.
pub const CONVERSATION_ACTIVITY_SCHEMA_VERSION: u16 = 1;

/// The kind of work described by an activity fact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityWorkKind {
    Command,
    Subagent,
}

/// Identity and ordering data every activity fact must carry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivityScope {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub host_epoch: HostEpoch,
    pub cursor: u64,
}

/// Immutable provider-neutral activity fact. It never contains transcript content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConversationActivityFact {
    TurnStarted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    ContextUsage {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        used_tokens: u64,
        window_tokens: Option<u64>,
    },
    RootActivity {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        activity: RootActivity,
    },
    RootPhase {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        phase: TurnPhase,
    },
    WorkPhase {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        work_id: String,
        kind: ActivityWorkKind,
        phase: WorkPhase,
    },
    ToolActivity {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        activity: ToolActivity,
    },
    SubagentStarted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        child_id: String,
        parent_tool_use_id: String,
    },
    DecisionPending {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        decision_id: String,
    },
    DecisionSettled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        decision_id: String,
    },
    InterruptRequested {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    Recovered {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    Terminal {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        phase: TurnPhase,
    },
}

impl ConversationActivityFact {
    /// Returns the common immutable scope carried by this fact.
    #[must_use]
    pub fn scope(&self) -> &ConversationActivityScope {
        match self {
            Self::TurnStarted { scope }
            | Self::ContextUsage { scope, .. }
            | Self::RootActivity { scope, .. }
            | Self::RootPhase { scope, .. }
            | Self::WorkPhase { scope, .. }
            | Self::ToolActivity { scope, .. }
            | Self::SubagentStarted { scope, .. }
            | Self::DecisionPending { scope, .. }
            | Self::DecisionSettled { scope, .. }
            | Self::InterruptRequested { scope }
            | Self::Recovered { scope }
            | Self::Terminal { scope, .. } => scope,
        }
    }
}

/// Bounded, cursor-ordered durable activity facts for one conversation run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivityPage {
    pub facts: Vec<ConversationActivityFact>,
    pub next_after_cursor: Option<u64>,
}

#[cfg(test)]
mod tests {

    use super::{ConversationActivityFact, ConversationActivityScope};
    use crate::HostEpoch;

    #[test]
    fn fact_round_trips_with_shared_scope() {
        let fact = ConversationActivityFact::DecisionPending {
            scope: ConversationActivityScope {
                conversation_id: "conversation-1".into(),
                run_id: "run-1".into(),
                turn_id: "turn-1".into(),
                host_epoch: HostEpoch(9),
                cursor: 12,
            },
            decision_id: "decision-1".into(),
        };

        let value = serde_json::to_value(&fact).expect("fact serializes");
        assert_eq!(value["type"], "decisionPending");
        assert_eq!(value["conversationId"], "conversation-1");
        assert_eq!(value["hostEpoch"], 9);
        assert_eq!(value["cursor"], 12);
        assert_eq!(
            serde_json::from_value::<ConversationActivityFact>(value).expect("fact deserializes"),
            fact
        );
    }
}
