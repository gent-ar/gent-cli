use crate::readiness_test_support as readiness;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatReadLedger,
    AgentChatWorkspaceLedger, ConversationLedger, Ledger,
};
use gent_types::*;

struct Fixture {
    ledger: gent_store::SqliteLedger,
    root: tempfile::TempDir,
    prompt: AgentChatPromptSaved,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let ledger = gent_store::SqliteLedger::open(root.path().join("fixture.db")).unwrap();
    let conversation = AgentChatConversationId("notice-conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("notice-run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "fixture".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "notice-workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation,
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "synthetic request".into(),
        })
        .unwrap();
    readiness::release(&ledger, &prompt);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                "notice-owner",
                HostEpoch(1),
                AgentChatProvider::Claude
            )
            .unwrap()
            .is_some()
    );
    ledger
        .claim_run_lease(&gent_ports::RunLease {
            run_id: "notice-run".into(),
            coordinator_id: "notice-owner".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&prompt.message.message_id, "notice-owner", HostEpoch(1))
        .unwrap();
    Fixture {
        ledger,
        root,
        prompt,
    }
}
fn state(f: &Fixture) -> String {
    let db = rusqlite::Connection::open_with_flags(
        f.root.path().join("fixture.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row(
        "SELECT state FROM agent_chat_prompt_dispatches WHERE message_id=?1",
        [&f.prompt.message.message_id],
        |row| row.get(0),
    )
    .unwrap()
}
fn notices(f: &Fixture) -> Vec<String> {
    f.ledger
        .read_agent_chat_transcript("notice-conversation", None, 32)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == gent_types::NormalizedTranscriptKind::Notice)
        .map(|event| event.text)
        .collect()
}
#[test]
fn notice_does_not_release_an_unknown_launch_or_its_lease() {
    let f = fixture();
    f.ledger
        .mark_agent_chat_prompt_unprovable(
            &f.prompt.message.message_id,
            "notice-owner",
            HostEpoch(1),
        )
        .unwrap();
    let lease = f.ledger.find_run_lease("notice-run").unwrap();
    f.ledger
        .record_agent_chat_prompt_failure_notice(
            &f.prompt.message.message_id,
            "notice-owner",
            HostEpoch(1),
            "Contained MCP configuration rejected before a usable session.",
            None,
        )
        .unwrap();
    assert_eq!(state(&f), "unprovable");
    assert_eq!(f.ledger.find_run_lease("notice-run").unwrap(), lease);
    assert_eq!(
        f.ledger.list_run_turns("notice-run").unwrap()[0].phase,
        DurableTurnPhase::Failed
    );
    assert!(
        f.ledger
            .claim_agent_chat_prompt_dispatch("new-owner", HostEpoch(1), AgentChatProvider::Claude)
            .unwrap()
            .is_none()
    );
    assert_eq!(notices(&f).len(), 1);
}
#[test]
fn stale_or_foreign_notice_cannot_write_or_replay_the_owned_launch() {
    let f = fixture();
    for (owner, epoch) in [("wrong", HostEpoch(1)), ("notice-owner", HostEpoch(2))] {
        assert!(
            f.ledger
                .record_agent_chat_prompt_failure_notice(
                    &f.prompt.message.message_id,
                    owner,
                    epoch,
                    "untrusted replacement",
                    None
                )
                .is_err()
        );
    }
    assert_eq!(state(&f), "launching");
    assert!(notices(&f).is_empty());
    assert!(
        f.ledger
            .claim_agent_chat_prompt_dispatch("new-owner", HostEpoch(1), AgentChatProvider::Claude)
            .unwrap()
            .is_none()
    );
}
#[test]
fn an_existing_explanation_is_preserved_without_state_transition() {
    let f = fixture();
    let id = &f.prompt.message.message_id;
    f.ledger
        .record_agent_chat_prompt_failure_notice(
            id,
            "notice-owner",
            HostEpoch(1),
            "specific original explanation",
            None,
        )
        .unwrap();
    f.ledger
        .record_agent_chat_prompt_failure_notice(
            id,
            "notice-owner",
            HostEpoch(1),
            "generic replacement",
            None,
        )
        .unwrap();
    assert_eq!(notices(&f), vec!["specific original explanation"]);
    assert_eq!(state(&f), "launching");
    f.ledger
        .confirm_agent_chat_prompt_started(id, "notice-owner", HostEpoch(1))
        .unwrap();
    assert!(
        f.ledger
            .record_agent_chat_prompt_failure_notice(
                id,
                "notice-owner",
                HostEpoch(1),
                "too late",
                None
            )
            .is_err()
    );
    assert_eq!(state(&f), "started");
}
#[test]
fn a_sandbox_notice_carries_its_failure_code_on_transcript_and_turn_follow_reads() {
    use gent_ports::TurnFollowReader;
    let f = fixture();
    f.ledger
        .record_agent_chat_prompt_failure_notice(
            &f.prompt.message.message_id,
            "notice-owner",
            HostEpoch(1),
            "Sandbox unavailable.",
            Some(AgentChatFailureCode::SandboxUnavailable),
        )
        .unwrap();
    let transcript = f
        .ledger
        .read_agent_chat_transcript("notice-conversation", None, 32)
        .unwrap()
        .events;
    let followed = f
        .ledger
        .turn_follow_page(
            "notice-conversation",
            "notice-run",
            &f.prompt.message.turn_id,
            0,
            32,
        )
        .unwrap()
        .events;
    for events in [transcript, followed] {
        let notice = events
            .iter()
            .find(|event| event.kind == NormalizedTranscriptKind::Notice)
            .unwrap();
        assert_eq!(
            notice.failure_code,
            Some(AgentChatFailureCode::SandboxUnavailable)
        );
    }
}
