use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, RunCheckpointRecord};

#[test]
fn coordinator_records_checkpoint_metadata_without_a_provider() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_run(&gent_core::Run {
            id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    let checkpoint = RunCheckpointRecord {
        checkpoint_id: "checkpoint-a".into(),
        run_id: "run-a".into(),
        sequence: 1,
        event_cursor: 7,
        state_digest_sha256: "b".repeat(64),
    };
    coordinator.save_run_checkpoint(&checkpoint).unwrap();
    assert_eq!(
        coordinator.run_checkpoints("run-a").unwrap(),
        vec![checkpoint]
    );
}
