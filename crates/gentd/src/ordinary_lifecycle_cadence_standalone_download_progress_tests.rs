use std::sync::Arc;

use gent_ports::{AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::super::{StandalonePromptRelease, StandaloneReadiness};
use super::models;
use crate::agent_chat_api::PromptWake;
use crate::local_model_download::{
    DownloadRequest, ModelDownloadError, ModelDownloadResponse, ModelDownloadTransport,
};

#[derive(Debug, Default)]
struct GatedTransport(Arc<Notify>);

#[derive(Debug)]
struct GatedResponse(Arc<Notify>, bool);

#[async_trait::async_trait]
impl ModelDownloadTransport for GatedTransport {
    async fn get(
        &self,
        _: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        Ok(Box::new(GatedResponse(Arc::clone(&self.0), false)))
    }
}

#[async_trait::async_trait]
impl ModelDownloadResponse for GatedResponse {
    fn status(&self) -> u16 {
        200
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        if !std::mem::replace(&mut self.1, true) {
            return Ok(Some(vec![1]));
        }
        self.0.notified().await;
        Err(ModelDownloadError::Transport("connection reset".into()))
    }
}

fn projected_activity(ledger: &SqliteLedger, conversation: &AgentChatConversationId) -> Vec<Value> {
    ledger
        .agent_chat_projection_page(conversation, 0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "activity")
        .map(|event| event.payload["activity"].clone())
        .collect()
}

async fn until(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    commits: &mut tokio::sync::watch::Receiver<u64>,
    kind: &str,
) -> Vec<Value> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let facts = projected_activity(ledger, conversation);
            if facts.iter().any(|fact| fact["type"] == kind) {
                return facts;
            }
            commits.changed().await.unwrap();
        }
    })
    .await
    .expect("the projection reaches the expected fact")
}

#[tokio::test]
async fn a_held_prompt_reports_its_model_download_to_followers_and_ends_with_a_terminal_phase() {
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
    let gate = Arc::new(Notify::new());
    let models = models(
        &ledger,
        directory.path(),
        Arc::new(GatedTransport(Arc::clone(&gate))),
    );
    let readiness = StandaloneReadiness::new(ledger.clone(), HostEpoch(1), Some(models));
    let mut commits = ledger.commits();
    readiness
        .provision_claurst(
            PromptWake {
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run".into()),
                receipt_id: saved.receipt.receipt_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
            },
            Arc::new(Notify::new()),
        )
        .unwrap();
    until(
        &ledger,
        &conversation_id,
        &mut commits,
        "modelDownloadProgress",
    )
    .await;
    gate.notify_one();
    let facts = until(&ledger, &conversation_id, &mut commits, "promptCanceled").await;
    let progress = facts
        .iter()
        .filter(|fact| fact["type"] == "modelDownloadProgress")
        .map(|fact| {
            json!({
                "modelId": fact["modelId"],
                "messageId": fact["messageId"],
                "receiptId": fact["receiptId"],
                "phase": fact["phase"],
                "downloadedBytes": fact["downloadedBytes"],
            })
        })
        .collect::<Vec<_>>();
    let expected = |phase: &str| {
        json!({
            "modelId": "qwen3-1-7b-q4-k-m",
            "messageId": saved.message.message_id,
            "receiptId": "prompt",
            "phase": phase,
            "downloadedBytes": 0,
        })
    };
    assert_eq!(progress, [expected("downloading"), expected("failed")]);
    let order = |kind: &str| facts.iter().position(|fact| fact["type"] == kind).unwrap();
    assert!(order("promptHeld") < order("modelDownloadProgress"));
    let failed = facts
        .iter()
        .rposition(|fact| fact["type"] == "modelDownloadProgress")
        .unwrap();
    assert!(failed < order("promptCanceled"));
    assert!(
        facts
            .iter()
            .filter(|fact| fact["type"] == "modelDownloadProgress")
            .all(|fact| fact["totalBytes"].as_u64().is_some_and(|total| total > 0))
    );
}
