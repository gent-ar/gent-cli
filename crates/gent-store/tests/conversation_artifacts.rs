use gent_ports::{ConversationArtifactLedger, ConversationLedger, RunRecord};
use gent_store::SqliteLedger;
use gent_types::{
    ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus, ConversationRecord,
};

fn artifact(id: &str, supersedes: Option<&str>) -> ConversationArtifact {
    ConversationArtifact {
        artifact_id: id.into(),
        conversation_id: "conversation".into(),
        kind: ConversationArtifactKind::Title,
        source_turn_ids: vec!["turn-1".into()],
        provider: "claude".into(),
        model_version: "1".into(),
        input_digest: "sha256:input".into(),
        status: ConversationArtifactStatus::Completed,
        text: Some("A durable title".into()),
        supersedes_artifact_id: supersedes.map(str::to_owned),
    }
}

#[test]
fn artifacts_preserve_provenance_and_supersession_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation".into(),
            },
            &RunRecord {
                run_id: "run".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    ledger
        .create_conversation_artifact(&artifact("title-1", None))
        .unwrap();
    ledger
        .create_conversation_artifact(&artifact("title-2", Some("title-1")))
        .unwrap();
    drop(ledger);
    let records = SqliteLedger::open(&path)
        .unwrap()
        .list_conversation_artifacts("conversation")
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].status, ConversationArtifactStatus::Superseded);
    assert_eq!(records[0].text, None);
    assert_eq!(
        records[1].supersedes_artifact_id.as_deref(),
        Some("title-1")
    );
    assert_eq!(records[0].source_turn_ids, vec!["turn-1"]);
}

#[test]
fn artifacts_reject_incomplete_or_cross_conversation_provenance() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let incomplete = artifact("bad", None);
    assert!(ledger.create_conversation_artifact(&incomplete).is_err());
}
