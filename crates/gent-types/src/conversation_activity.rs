//! Typed, content-free activity facts for a conversation projection.

use serde::{Deserialize, Serialize};

use crate::{HostEpoch, RootActivity, TurnPhase, WorkPhase};

/// Version of the conversation-activity value contract.
pub const CONVERSATION_ACTIVITY_SCHEMA_VERSION: u16 = 1;

/// A coarse activity state suitable for a client loading or waiting indicator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationActivityState {
    Idle,
    Thinking,
    WaitingForCommand,
    WaitingForSubagents,
    AwaitingUser,
    Interrupting,
    Recovering,
}

/// The independently tracked kind of live work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityWorkKind {
    Command,
    Subagent,
}

/// Progress of a single content-free unit of conversation work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityWork {
    pub id: String,
    pub kind: ActivityWorkKind,
    pub phase: WorkPhase,
}

/// A complete, volatile activity snapshot. It never contains transcript content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivity {
    pub schema_version: u16,
    pub conversation_id: String,
    pub run_id: String,
    pub host_epoch: HostEpoch,
    pub revision: u64,
    pub activity_sequence: u64,
    pub cursor: u64,
    pub active_turn_id: Option<String>,
    pub root_phase: TurnPhase,
    pub state: ConversationActivityState,
    pub pending_decision_id: Option<String>,
    pub work: Vec<ActivityWork>,
    pub has_error: bool,
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

/// Typed coordinator input used to derive a [`ConversationActivity`] snapshot.
///
/// The reducer belongs to a higher-level crate; this shared contract has no side effects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConversationActivityFact {
    TurnStarted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ActivityWork, ActivityWorkKind, CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivity,
        ConversationActivityFact, ConversationActivityScope, ConversationActivityState,
    };
    use crate::{HostEpoch, TurnPhase, WorkPhase};

    #[test]
    fn snapshot_serializes_as_a_stable_camel_case_value() {
        let activity = ConversationActivity {
            schema_version: CONVERSATION_ACTIVITY_SCHEMA_VERSION,
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            host_epoch: HostEpoch(9),
            revision: 3,
            activity_sequence: 7,
            cursor: 12,
            active_turn_id: Some("turn-1".into()),
            root_phase: TurnPhase::Processing,
            state: ConversationActivityState::WaitingForSubagents,
            pending_decision_id: None,
            work: vec![ActivityWork {
                id: "child-1".into(),
                kind: ActivityWorkKind::Subagent,
                phase: WorkPhase::Running,
            }],
            has_error: false,
        };

        assert_eq!(
            serde_json::to_value(activity).expect("activity serializes"),
            json!({
                "schemaVersion": 1,
                "conversationId": "conversation-1",
                "runId": "run-1",
                "hostEpoch": 9,
                "revision": 3,
                "activitySequence": 7,
                "cursor": 12,
                "activeTurnId": "turn-1",
                "rootPhase": "processing",
                "state": "waitingForSubagents",
                "pendingDecisionId": null,
                "work": [{ "id": "child-1", "kind": "subagent", "phase": "running" }],
                "hasError": false,
            })
        );
    }

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
