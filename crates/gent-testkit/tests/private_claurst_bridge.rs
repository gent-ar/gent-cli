use gent_ports::{
    ClaurstCheckpoint, ClaurstDrainBatch, ClaurstDrainRequest, ClaurstFactValue,
    ClaurstNormalizedFact, ClaurstSessionBinding, ClaurstSourceId, ClaurstTerminal,
    PrivateClaurstBridge,
};
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::NormalizedProviderEvent;

fn source() -> ClaurstSourceId {
    ClaurstSourceId("source-a".into())
}

fn binding() -> ClaurstSessionBinding {
    ClaurstSessionBinding {
        run_id: "run-a".into(),
        source_id: source(),
        opaque_session_id: "private-session-a".into(),
    }
}

fn request(after_cursor: u64, limit: u16) -> ClaurstDrainRequest {
    ClaurstDrainRequest {
        run_id: "run-a".into(),
        source_id: source(),
        after_cursor,
        limit,
    }
}

#[tokio::test]
async fn bridge_drains_ordered_normalized_facts_and_terminal_once() {
    let bridge = FakePrivateClaurstBridge::default();
    bridge.bind_session(binding()).await.unwrap();
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: source(),
            cursor: 4,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::Output {
                text: "safe normalized reply".into(),
                is_partial: false,
            }),
        }],
        checkpoint: Some(ClaurstCheckpoint {
            run_id: "run-a".into(),
            source_id: source(),
            cursor: 4,
            state_digest_sha256: "digest".into(),
        }),
        session_binding: None,
        terminal: Some(ClaurstTerminal::Completed),
    });

    let batch = bridge.drain(request(3, 1)).await.unwrap();
    assert_eq!(batch.facts.len(), 1);
    assert!(matches!(batch.terminal, Some(ClaurstTerminal::Completed)));
    assert_eq!(bridge.requests(), vec![request(3, 1)]);
    assert!(bridge.drain(request(4, 1)).await.is_err());
}

#[tokio::test]
async fn bridge_rejects_unbounded_or_opaque_session_echoes() {
    let bridge = FakePrivateClaurstBridge::default();
    bridge.bind_session(binding()).await.unwrap();
    assert!(bridge.drain(request(0, 65)).await.is_err());

    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: source(),
            cursor: 1,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::Output {
                text: "private-session-a".into(),
                is_partial: false,
            }),
        }],
        checkpoint: None,
        session_binding: None,
        terminal: None,
    });
    assert!(bridge.drain(request(0, 1)).await.is_err());
}
