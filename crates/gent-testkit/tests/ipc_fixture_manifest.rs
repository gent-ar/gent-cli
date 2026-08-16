use std::path::{Path, PathBuf};

use gent_testkit::validate_ipc_fixture_manifest;
use tempfile::TempDir;

const FIXTURES: [&str; 7] = [
    "manifest.json",
    "handshake.json",
    "core.json",
    "event-stream.json",
    "agent-chat-conversations.json",
    "agent-chat-transcript.json",
    "agent-chat-intents.json",
];

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ipc-contract")
}

fn copied_fixtures() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    for name in FIXTURES {
        std::fs::copy(source_root().join(name), directory.path().join(name)).unwrap();
    }
    directory
}

#[test]
fn repository_ipc_fixture_contract_is_valid() {
    validate_ipc_fixture_manifest(&source_root().join("manifest.json")).unwrap();
}

#[test]
fn agent_chat_contract_cannot_be_declared_composed() {
    let directory = copied_fixtures();
    let path = directory.path().join("agent-chat-intents.json");
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, content.replacen("\"reserved\"", "\"composed\"", 1)).unwrap();
    let error = validate_ipc_fixture_manifest(&directory.path().join("manifest.json")).unwrap_err();
    assert!(error.contains("must remain reserved"));
}

#[test]
fn noncanonical_wire_json_is_rejected() {
    let directory = copied_fixtures();
    let path = directory.path().join("handshake.json");
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        path,
        content.replacen(
            "\"protocolMax\": 1",
            "\"protocolMax\": 1, \"extra\": true",
            1,
        ),
    )
    .unwrap();
    let error = validate_ipc_fixture_manifest(&directory.path().join("manifest.json")).unwrap_err();
    assert!(error.contains("canonical public JSON"));
}
