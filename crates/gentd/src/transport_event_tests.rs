use gent_types::{EventResume, EventSnapshot, HostEpoch};
use serde_json::json;

use crate::transport::event_frame;

#[test]
fn stale_event_feeds_require_the_explicit_resync_capability() {
    let resume = EventResume::Resync {
        snapshot: EventSnapshot {
            cursor: 4,
            host_epoch: HostEpoch(1),
            schema_version: 1,
            payload: json!({ "safe": true }),
        },
        events: Vec::new(),
    };
    assert!(event_frame(resume.clone(), false).is_err());
    assert!(matches!(
        event_frame(resume, true),
        Ok(gent_protocol::WireFrame::EventResync { .. })
    ));
}
