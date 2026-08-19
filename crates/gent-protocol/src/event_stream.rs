//! Capability-gated cursor stream frames kept separate from finite event pages.

use gent_types::EventPage;
use serde::{Deserialize, Serialize};

/// Negotiates the long-lived, read-only local event stream.
pub const EVENT_STREAM_CAPABILITY: &str = "event-stream-v1";

/// Frames used only after a negotiated [`EVENT_STREAM_CAPABILITY`] handshake.
///
/// `Replay` is the initial bounded durable page and `Events` contains later
/// cursor-ordered pages. `Ack` is client-owned progress: it never changes daemon
/// retention or skips a durable event.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum EventStreamFrame {
    Attach { after_cursor: u64 },
    Replay { page: EventPage },
    Events { page: EventPage },
    Ack { cursor: u64 },
    Error { code: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::{EVENT_STREAM_CAPABILITY, EventStreamFrame};
    use gent_types::{Event, EventPage, HostEpoch, ReceiptId};

    #[test]
    fn stream_frames_have_a_distinct_discriminant() {
        let frame = EventStreamFrame::Replay {
            page: EventPage {
                events: vec![Event {
                    cursor: 3,
                    event_id: "event-3".into(),
                    receipt_id: ReceiptId("receipt-3".into()),
                    host_epoch: HostEpoch(1),
                    kind: "accepted".into(),
                    payload: serde_json::json!({}),
                }],
                next_after_cursor: None,
            },
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["type"], "replay");
        assert_eq!(EVENT_STREAM_CAPABILITY, "event-stream-v1");
        assert_eq!(
            serde_json::from_value::<EventStreamFrame>(value).unwrap(),
            frame
        );
    }
}
