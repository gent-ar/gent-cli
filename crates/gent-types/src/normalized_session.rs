//! Provider-neutral values for one atomically persisted normalized session batch.

use serde::{Deserialize, Serialize};

use crate::{
    ConversationActivityFact, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent,
    NormalizedTranscriptAppend,
};

/// The lifecycle component of a daemon-normalized session batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NormalizedSessionLifecycle {
    Event { event: NormalizedProviderEvent },
    Signal { signal: NormalizedLifecycleSignal },
}

/// Exact daemon-owned binding and optional projections for one normalized source fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedSessionBatch {
    pub coordinator_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub host_epoch: HostEpoch,
    pub lifecycle_event_id: String,
    pub lifecycle: NormalizedSessionLifecycle,
    pub transcript: Option<NormalizedTranscriptAppend>,
    pub activity_event_id: Option<String>,
    pub activity: Option<ConversationActivityFact>,
}

/// All cursors committed by one idempotent normalized-session transaction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedSessionBatchResult {
    pub lifecycle_cursor: u64,
    pub transcript_cursor: Option<u64>,
    pub activity_cursor: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{NormalizedSessionBatch, NormalizedSessionLifecycle};
    use crate::{HostEpoch, NormalizedProviderEvent};

    #[test]
    fn batch_serializes_without_provider_native_transport_fields() {
        let batch = NormalizedSessionBatch {
            coordinator_id: "daemon-1".into(),
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            host_epoch: HostEpoch(7),
            lifecycle_event_id: "lifecycle-1".into(),
            lifecycle: NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::TurnStarted {
                    turn_id: "turn-1".into(),
                },
            },
            transcript: None,
            activity_event_id: None,
            activity: None,
        };
        let value = serde_json::to_value(batch).unwrap();
        assert_eq!(value["lifecycle"]["type"], "event");
        assert!(value.get("providerPayload").is_none());
    }
}
