//! Typed, content-free activity facts for one conversation run.

use serde::{Deserialize, Serialize};

use crate::{
    GoalRecord, HostEpoch, PlanArtifact, PromptHoldReason, RootActivity, TokenUsage, ToolActivity,
    TurnPhase, WorkPhase,
};

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnTerminalCause {
    Steered,
    DeliveryUnprovable,
    ProviderSessionUnavailable,
}

/// Immutable provider-neutral activity fact. It never contains transcript content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConversationActivityFact {
    TurnStarted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    ContextUsage {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "used_tokens")]
        used_tokens: u64,
        #[serde(alias = "window_tokens")]
        window_tokens: Option<u64>,
    },
    TokenUsage {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        usage: TokenUsage,
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
        #[serde(alias = "work_id")]
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
        #[serde(alias = "child_id")]
        child_id: String,
        #[serde(alias = "parent_tool_use_id")]
        parent_tool_use_id: String,
    },
    DecisionPending {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "decision_id")]
        decision_id: String,
    },
    DecisionSettled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "decision_id")]
        decision_id: String,
    },
    PromptQueued {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
    },
    PromptHeld {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        reason: PromptHoldReason,
    },
    PromptReleased {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
    },
    PromptCanceled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
    },
    PromptSteered {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        transcript_cursor: u64,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<TurnTerminalCause>,
    },
    GoalUpdated {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        goal: GoalRecord,
    },
    PlanUpdated {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        plan: PlanArtifact,
    },
}

impl ConversationActivityFact {
    /// Returns the common immutable scope carried by this fact.
    #[must_use]
    pub fn scope(&self) -> &ConversationActivityScope {
        match self {
            Self::TurnStarted { scope }
            | Self::ContextUsage { scope, .. }
            | Self::TokenUsage { scope, .. }
            | Self::RootActivity { scope, .. }
            | Self::RootPhase { scope, .. }
            | Self::WorkPhase { scope, .. }
            | Self::ToolActivity { scope, .. }
            | Self::SubagentStarted { scope, .. }
            | Self::DecisionPending { scope, .. }
            | Self::DecisionSettled { scope, .. }
            | Self::PromptQueued { scope, .. }
            | Self::PromptHeld { scope, .. }
            | Self::PromptReleased { scope, .. }
            | Self::PromptCanceled { scope, .. }
            | Self::PromptSteered { scope, .. }
            | Self::InterruptRequested { scope }
            | Self::Recovered { scope }
            | Self::Terminal { scope, .. }
            | Self::GoalUpdated { scope, .. }
            | Self::PlanUpdated { scope, .. } => scope,
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
        assert_eq!(value["decisionId"], "decision-1");
        assert!(value.get("decision_id").is_none());
        assert_eq!(
            serde_json::from_value::<ConversationActivityFact>(value).expect("fact deserializes"),
            fact
        );
    }

    #[test]
    fn a_terminal_cause_is_additive_and_absent_for_ordinary_endings() {
        let scope = ConversationActivityScope {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            host_epoch: HostEpoch(1),
            cursor: 3,
        };
        let steered = ConversationActivityFact::Terminal {
            scope: scope.clone(),
            phase: crate::TurnPhase::Interrupted,
            cause: Some(crate::TurnTerminalCause::Steered),
        };
        let value = serde_json::to_value(&steered).unwrap();
        assert_eq!(
            (value["phase"].clone(), value["cause"].clone()),
            ("interrupted".into(), "steered".into())
        );
        let stopped = serde_json::json!({
            "type": "terminal", "conversationId": "conversation-1", "runId": "run-1",
            "turnId": "turn-1", "hostEpoch": 1, "cursor": 3, "phase": "interrupted"
        });
        let parsed = serde_json::from_value::<ConversationActivityFact>(stopped.clone()).unwrap();
        assert!(matches!(
            parsed,
            ConversationActivityFact::Terminal { cause: None, .. }
        ));
        assert_eq!(serde_json::to_value(parsed).unwrap(), stopped);
    }

    #[test]
    fn held_prompt_fact_names_its_prompt_and_typed_reason_without_secrets() {
        let fact = ConversationActivityFact::PromptHeld {
            scope: ConversationActivityScope {
                conversation_id: "conversation-1".into(),
                run_id: "run-1".into(),
                turn_id: "turn-1".into(),
                host_epoch: HostEpoch(9),
                cursor: 12,
            },
            message_id: "message-1".into(),
            receipt_id: "prompt-receipt".into(),
            reason: crate::PromptHoldReason::ModelDownload,
        };

        assert_eq!(
            serde_json::to_value(&fact).expect("fact serializes"),
            serde_json::json!({
                "type": "promptHeld",
                "conversationId": "conversation-1",
                "runId": "run-1",
                "turnId": "turn-1",
                "hostEpoch": 9,
                "cursor": 12,
                "messageId": "message-1",
                "receiptId": "prompt-receipt",
                "reason": "modelDownload"
            })
        );
        assert_eq!(
            serde_json::from_value::<ConversationActivityFact>(
                serde_json::to_value(&fact).expect("fact serializes")
            )
            .expect("fact deserializes"),
            fact
        );
    }
}
