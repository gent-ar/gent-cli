use std::path::Path;

use gent_testkit::load_public_driver_fixtures;

#[test]
fn every_committed_public_driver_recording_is_hygienic_and_replayable() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/public-driver-transcripts");
    let recordings = load_public_driver_fixtures(root).unwrap();
    assert!(!recordings.is_empty());
    for recording in recordings {
        assert_eq!(recording.metadata["status"], "recorded");
        assert!(!recording.frames.is_empty());
    }
}
