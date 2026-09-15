use std::{sync::Arc, time::Duration};

use gent_ports::{AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger};
use gent_ports::{ConversationActivityLedger, ConversationLedger};
use gent_protocol::{LocalModelDownloadFailure, LocalModelFrame};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ConversationActivityFact, DurableTurnPhase, HostEpoch,
    ReceiptId, TurnPhase, WorkspaceRecord,
};
use tokio::sync::Notify;

use super::super::{StandalonePromptRelease, StandaloneReadiness};
use crate::{
    agent_chat_api::PromptWake,
    local_model_events::LocalModelEvents,
    local_model_jobs::tests::{BlockingTransport, download_frames, eventually},
    runtime_facade::model_catalog::downloads::tests::downloading,
    standalone_authority_composition::StandaloneClaurstModels,
};

const MODEL: &str = "qwen3-8b-q4-k-m";

struct Shared {
    _directory: tempfile::TempDir,
    ledger: SqliteLedger,
    models: StandaloneClaurstModels,
    readiness: StandaloneReadiness,
    owner: Waiting,
    follower: Waiting,
}

struct Waiting {
    conversation: &'static str,
    run: &'static str,
    receipt: &'static str,
    turn_id: String,
}

fn prompt(
    ledger: &SqliteLedger,
    conversation: &'static str,
    run: &'static str,
    receipt: &'static str,
) -> Waiting {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("create-{conversation}")),
                idempotency_key: format!("create-{conversation}"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation.into()),
                run_id: AgentChatRunId(run.into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: MODEL.into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    Waiting {
        conversation,
        run,
        receipt,
        turn_id: send(ledger, conversation, receipt).message.turn_id,
    }
}

fn send(
    ledger: &SqliteLedger,
    conversation: &str,
    receipt: &str,
) -> gent_types::AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{receipt}")),
            receipt_id: ReceiptId(receipt.into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(conversation.into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap()
}

fn wake(waiting: &Waiting) -> PromptWake {
    PromptWake {
        conversation_id: AgentChatConversationId(waiting.conversation.into()),
        run_id: AgentChatRunId(waiting.run.into()),
        receipt_id: ReceiptId(waiting.receipt.into()),
        disposition: AgentChatPromptDisposition::Send,
    }
}

fn restored(ledger: &SqliteLedger, waiting: &Waiting) -> Vec<ConversationActivityFact> {
    ledger
        .read_conversation_activity_page(waiting.conversation, waiting.run, 0, 50)
        .unwrap()
        .facts
}

fn projected(ledger: &SqliteLedger, waiting: &Waiting) -> Vec<ConversationActivityFact> {
    ledger
        .agent_chat_projection_page(
            &AgentChatConversationId(waiting.conversation.into()),
            0,
            100,
        )
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "activity")
        .map(|event| serde_json::from_value(event.payload["activity"].clone()).unwrap())
        .collect()
}

fn count(facts: &[ConversationActivityFact], name: &str) -> usize {
    facts
        .iter()
        .filter(|fact| serde_json::to_value(fact).unwrap()["type"] == name)
        .count()
}

fn phase(ledger: &SqliteLedger, waiting: &Waiting) -> DurableTurnPhase {
    ledger.find_turn(&waiting.turn_id).unwrap().unwrap().phase
}

fn assert_cancelled(ledger: &SqliteLedger, waiting: &Waiting) {
    assert_eq!(phase(ledger, waiting), DurableTurnPhase::Cancelled);
    for facts in [restored(ledger, waiting), projected(ledger, waiting)] {
        assert_eq!(count(&facts, "promptHeld"), 1);
        assert_eq!(count(&facts, "promptCanceled"), 1);
        assert_eq!(count(&facts, "terminal"), 1);
        assert!(facts.iter().any(|fact| matches!(
            fact,
            ConversationActivityFact::Terminal {
                phase: TurnPhase::Cancelled,
                ..
            }
        )));
    }
}

fn assert_still_waiting(ledger: &SqliteLedger, waiting: &Waiting) {
    assert_eq!(phase(ledger, waiting), DurableTurnPhase::Active);
    let facts = restored(ledger, waiting);
    assert_eq!(count(&facts, "promptHeld"), 1);
    assert_eq!(count(&facts, "promptCanceled"), 0);
    assert_eq!(count(&facts, "terminal"), 0);
    assert!(
        !download_frames(ledger)
            .iter()
            .any(|frame| matches!(frame, LocalModelFrame::DownloadFailed { .. }))
    );
}

async fn shared_download() -> Shared {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let owner = prompt(&ledger, "conversation-owner", "run-owner", "owner");
    let follower = prompt(&ledger, "conversation-follower", "run-follower", "follower");
    let models = StandaloneClaurstModels::from_data_dir(
        directory.path(),
        LocalModelEvents::new(ledger.clone(), HostEpoch(1)),
    )
    .unwrap()
    .with_download_transport(Arc::new(BlockingTransport));
    let partial = models.provisioner.plan(MODEL).unwrap().partial_destination;
    let readiness = StandaloneReadiness::new(ledger.clone(), HostEpoch(1), Some(models.clone()));
    readiness
        .provision_claurst(wake(&owner), Arc::new(Notify::new()))
        .unwrap();
    eventually(|| downloading(&models, MODEL) && partial.exists()).await;
    readiness
        .provision_claurst(wake(&follower), Arc::new(Notify::new()))
        .unwrap();
    eventually(|| count(&restored(&ledger, &follower), "promptHeld") == 1).await;
    Shared {
        _directory: directory,
        ledger,
        models,
        readiness,
        owner,
        follower,
    }
}

#[tokio::test]
async fn canceling_a_follower_leaves_the_owner_download_running_and_its_conversation_usable() {
    let shared = shared_download().await;

    assert!(
        shared
            .readiness
            .cancel_held_prompts(shared.follower.run)
            .unwrap()
    );

    assert_cancelled(&shared.ledger, &shared.follower);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(downloading(&shared.models, MODEL));
    assert_still_waiting(&shared.ledger, &shared.owner);
    assert_eq!(
        count(
            &restored(&shared.ledger, &shared.follower),
            "promptCanceled"
        ),
        1
    );
    send(
        &shared.ledger,
        shared.follower.conversation,
        "follower-again",
    );
    assert!(
        !shared
            .readiness
            .cancel_held_prompts(shared.follower.run)
            .unwrap()
    );
}

#[tokio::test]
async fn canceling_the_owner_keeps_the_download_for_a_parked_follower_until_nobody_waits() {
    let shared = shared_download().await;

    assert!(
        shared
            .readiness
            .cancel_held_prompts(shared.owner.run)
            .unwrap()
    );

    assert_cancelled(&shared.ledger, &shared.owner);
    let frames = download_frames(&shared.ledger).len();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(downloading(&shared.models, MODEL));
    assert_still_waiting(&shared.ledger, &shared.follower);
    assert_eq!(download_frames(&shared.ledger).len(), frames);

    assert!(
        shared
            .readiness
            .cancel_held_prompts(shared.follower.run)
            .unwrap()
    );

    assert_cancelled(&shared.ledger, &shared.follower);
    eventually(|| !downloading(&shared.models, MODEL)).await;
    eventually(|| {
        matches!(
            download_frames(&shared.ledger).last(),
            Some(LocalModelFrame::DownloadFailed {
                reason: LocalModelDownloadFailure::Cancelled,
                ..
            })
        )
    })
    .await;
}
