use std::sync::Arc;

use async_trait::async_trait;
use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationActivityLedger,
    ConversationLedger, Ledger, TranscriptLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};
use tokio::sync::Notify;

use super::{StandalonePromptRelease, StandaloneReadiness};
use crate::{
    agent_chat_api::PromptWake,
    local_model_download::{
        DownloadRequest, ModelDownloadError, ModelDownloadResponse, ModelDownloadTransport,
    },
    local_model_events::LocalModelEvents,
    local_model_jobs::tests::BlockingTransport,
    runtime_facade::model_catalog::downloads::tests::downloading,
    standalone_authority_composition::StandaloneClaurstModels,
};

fn models(
    ledger: &SqliteLedger,
    directory: &std::path::Path,
    transport: Arc<dyn ModelDownloadTransport>,
) -> StandaloneClaurstModels {
    StandaloneClaurstModels::from_data_dir(
        directory,
        LocalModelEvents::new(ledger.clone(), HostEpoch(1)),
    )
    .unwrap()
    .with_download_transport(transport)
}

#[derive(Debug)]
struct Transport;

#[derive(Debug)]
struct Response(Option<Vec<u8>>);

#[async_trait]
impl ModelDownloadTransport for Transport {
    async fn get(
        &self,
        _: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        Ok(Box::new(Response(Some(vec![1]))))
    }
}

#[async_trait]
impl ModelDownloadResponse for Response {
    fn status(&self) -> u16 {
        200
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        Ok(self.0.take())
    }
}

#[tokio::test]
async fn queued_claurst_readiness_cannot_capture_the_run_interrupt() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let conversation_id = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-8b-q4-k-m".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: directory.path().display().to_string(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Queue,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let models = models(&ledger, directory.path(), Arc::new(BlockingTransport));
    let readiness = StandaloneReadiness::new(ledger, HostEpoch(1), Some(models));

    readiness
        .provision_claurst(
            PromptWake {
                conversation_id,
                run_id: AgentChatRunId("run".into()),
                receipt_id: saved.receipt.receipt_id,
                disposition: AgentChatPromptDisposition::Queue,
            },
            Arc::new(Notify::new()),
        )
        .unwrap();

    assert!(!readiness.cancel_held_prompts("run").unwrap());
}

#[tokio::test]
async fn automatic_claurst_provision_persists_progress_for_event_replay() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let conversation_id = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-1-7b-q4-k-m".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: directory.path().display().to_string(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let models = models(&ledger, directory.path(), Arc::new(Transport));
    let readiness = StandaloneReadiness::new(ledger.clone(), HostEpoch(1), Some(models));

    readiness
        .provision_claurst(
            PromptWake {
                conversation_id,
                run_id: AgentChatRunId("run".into()),
                receipt_id: saved.receipt.receipt_id,
                disposition: gent_types::AgentChatPromptDisposition::Send,
            },
            Arc::new(Notify::new()),
        )
        .unwrap();

    let events = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let events = ledger.read_event_page(0, 10).unwrap().events;
            if events
                .iter()
                .filter(|event| event.kind == "localModelDownload")
                .count()
                >= 3
            {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let frames = events
        .iter()
        .filter(|event| event.kind == "localModelDownload")
        .map(|event| {
            serde_json::from_value::<gent_protocol::LocalModelFrame>(event.payload.clone()).unwrap()
        })
        .collect::<Vec<_>>();
    assert!(frames.iter().all(|frame| match frame {
        gent_protocol::LocalModelFrame::DownloadAccepted { request_id, .. }
        | gent_protocol::LocalModelFrame::DownloadProgress { request_id, .. }
        | gent_protocol::LocalModelFrame::DownloadComplete { request_id, .. }
        | gent_protocol::LocalModelFrame::DownloadFailed { request_id, .. } => {
            request_id.starts_with("model-download-")
        }
        _ => false,
    }));
    assert!(matches!(
        frames[0],
        gent_protocol::LocalModelFrame::DownloadAccepted { .. }
    ));
    assert!(frames.iter().any(|frame| matches!(
        frame,
        gent_protocol::LocalModelFrame::DownloadProgress {
            downloaded_bytes: 0,
            ..
        }
    )));
    assert!(matches!(
        frames.last(),
        Some(gent_protocol::LocalModelFrame::DownloadFailed { .. })
    ));
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        gent_types::DurableTurnPhase::Failed
    );
    let transcript = ledger
        .normalized_transcript_page(
            &gent_types::AgentChatConversationId("conversation".into()),
            0,
            10,
        )
        .unwrap();
    assert_eq!(transcript.events.len(), 2);
    assert_eq!(
        transcript.events[0].kind,
        gent_types::NormalizedTranscriptKind::UserMessage
    );
    assert_eq!(
        transcript.events[1].kind,
        gent_types::NormalizedTranscriptKind::Notice
    );
}

#[tokio::test]
async fn a_prompt_parked_behind_a_running_download_is_held_for_every_client() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let conversation_id = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-8b-q4-k-m".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: directory.path().display().to_string(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let models = models(&ledger, directory.path(), Arc::new(BlockingTransport));
    models
        .downloads
        .start(
            "qwen3-8b-q4-k-m",
            crate::local_model_jobs::DownloadHolder::Background,
        )
        .unwrap();
    assert!(downloading(&models, "qwen3-8b-q4-k-m"));

    let readiness = StandaloneReadiness::new(ledger.clone(), HostEpoch(1), Some(models.clone()));
    let prompt = PromptWake {
        conversation_id: conversation_id.clone(),
        run_id: AgentChatRunId("run".into()),
        receipt_id: saved.receipt.receipt_id.clone(),
        disposition: AgentChatPromptDisposition::Send,
    };
    readiness
        .provision_claurst(prompt.clone(), Arc::new(Notify::new()))
        .unwrap();

    let held = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let facts = ledger
                .read_conversation_activity_page("conversation", "run", 0, 20)
                .unwrap()
                .facts;
            if facts.iter().any(|fact| {
                matches!(
                    fact,
                    gent_types::ConversationActivityFact::PromptHeld { .. }
                )
            }) {
                return facts;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a prompt parked behind a running download is held");
    assert_eq!(count_of(&held, "promptHeld"), 1);
    assert!(held.iter().any(|fact| matches!(
        fact,
        gent_types::ConversationActivityFact::PromptHeld {
            reason: gent_types::PromptHoldReason::ModelDownload,
            message_id,
            ..
        } if *message_id == saved.message.message_id
    )));
    assert_eq!(count_of(&projected(&ledger), "promptHeld"), 1);

    readiness.release(&prompt).unwrap();
    let cleared = ledger
        .read_conversation_activity_page("conversation", "run", 0, 20)
        .unwrap()
        .facts;
    assert_eq!(count_of(&cleared, "promptReleased"), 1);
    assert_eq!(count_of(&cleared, "promptCanceled"), 0);
    assert_eq!(count_of(&projected(&ledger), "promptReleased"), 1);
}

fn projected(ledger: &SqliteLedger) -> Vec<gent_types::ConversationActivityFact> {
    gent_ports::AgentChatProjectionLedger::agent_chat_projection_page(
        ledger,
        &AgentChatConversationId("conversation".into()),
        0,
        50,
    )
    .unwrap()
    .events
    .into_iter()
    .filter(|event| event.kind == "activity")
    .map(|event| serde_json::from_value(event.payload["activity"].clone()).unwrap())
    .collect()
}

fn count_of(facts: &[gent_types::ConversationActivityFact], name: &str) -> usize {
    facts
        .iter()
        .filter(|fact| serde_json::to_value(fact).unwrap()["type"] == name)
        .count()
}

#[tokio::test]
async fn canceling_claurst_download_cancels_the_prompt_without_leaving_an_active_download() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let conversation_id = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-8b-q4-k-m".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: directory.path().display().to_string(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let models = models(&ledger, directory.path(), Arc::new(BlockingTransport));
    let readiness = StandaloneReadiness::new(ledger.clone(), HostEpoch(1), Some(models.clone()));

    readiness
        .provision_claurst(
            PromptWake {
                conversation_id,
                run_id: AgentChatRunId("run".into()),
                receipt_id: saved.receipt.receipt_id,
                disposition: AgentChatPromptDisposition::Send,
            },
            Arc::new(Notify::new()),
        )
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let started = ledger
                .read_event_page(0, 10)
                .unwrap()
                .events
                .into_iter()
                .filter(|event| event.kind == "localModelDownload")
                .map(|event| {
                    serde_json::from_value::<gent_protocol::LocalModelFrame>(event.payload).unwrap()
                })
                .any(|frame| {
                    matches!(
                        frame,
                        gent_protocol::LocalModelFrame::DownloadAccepted { .. }
                    )
                });
            if started {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(readiness.cancel_held_prompts("run").unwrap());

    let frames = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let frames = ledger
                .read_event_page(0, 10)
                .unwrap()
                .events
                .into_iter()
                .filter(|event| event.kind == "localModelDownload")
                .map(|event| {
                    serde_json::from_value::<gent_protocol::LocalModelFrame>(event.payload).unwrap()
                })
                .collect::<Vec<_>>();
            if matches!(
                frames.last(),
                Some(gent_protocol::LocalModelFrame::DownloadFailed { .. })
            ) {
                return frames;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        frames.last(),
        Some(gent_protocol::LocalModelFrame::DownloadFailed {
            reason: gent_protocol::LocalModelDownloadFailure::Cancelled,
            ..
        })
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while downloading(&models, "qwen3-8b-q4-k-m") {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("nobody waits, so the download stops");
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        gent_types::DurableTurnPhase::Cancelled
    );
    let facts = ledger
        .read_conversation_activity_page(&saved.message.conversation_id, &saved.run_id.0, 0, 20)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|fact| matches!(
        fact,
        gent_types::ConversationActivityFact::PromptHeld {
            reason: gent_types::PromptHoldReason::ModelDownload,
            message_id,
            ..
        } if *message_id == saved.message.message_id
    )));
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(
                fact,
                gent_types::ConversationActivityFact::PromptCanceled { .. }
            ))
            .count(),
        1
    );
    assert!(facts.iter().any(|fact| matches!(
        fact,
        gent_types::ConversationActivityFact::Terminal {
            phase: gent_types::TurnPhase::Cancelled,
            ..
        }
    )));
}
