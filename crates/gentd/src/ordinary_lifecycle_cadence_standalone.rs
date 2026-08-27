use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use gent_ports::AgentChatPromptDispatchLedger;
use gent_protocol::{LocalModelFrame, LocalModelInstallState};
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatPromptDisposition, AgentChatProvider, HostEpoch, ProviderPromptReadinessBinding,
    ProviderPromptReadinessFailureBinding,
};
use tokio::sync::{Notify, oneshot};

use super::PromptWake;

#[derive(Clone)]
pub(super) struct StandaloneReadiness {
    reads: AgentChatReadService<SqliteLedger>,
    dispatches: SqliteLedger,
    host_epoch: HostEpoch,
    models: Option<crate::standalone_authority_composition::StandaloneClaurstModels>,
    transport: Arc<dyn crate::local_model_download::ModelDownloadTransport>,
    cancellations: Arc<Mutex<BTreeMap<String, oneshot::Sender<()>>>>,
}

struct ProvisionCancellation {
    values: Arc<Mutex<BTreeMap<String, oneshot::Sender<()>>>>,
    run_id: String,
}

impl Drop for ProvisionCancellation {
    fn drop(&mut self) {
        if let Ok(mut values) = self.values.lock() {
            values.remove(&self.run_id);
        }
    }
}

impl StandaloneReadiness {
    pub(super) fn new(
        ledger: SqliteLedger,
        host_epoch: HostEpoch,
        models: Option<crate::standalone_authority_composition::StandaloneClaurstModels>,
    ) -> Self {
        Self {
            reads: AgentChatReadService::new(ledger.clone()),
            dispatches: ledger,
            host_epoch,
            models,
            transport: Arc::new(crate::local_model_download::ReqwestModelDownloadTransport::new()),
            cancellations: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    #[cfg(test)]
    fn with_transport(
        ledger: SqliteLedger,
        host_epoch: HostEpoch,
        models: crate::standalone_authority_composition::StandaloneClaurstModels,
        transport: Arc<dyn crate::local_model_download::ModelDownloadTransport>,
    ) -> Self {
        Self {
            reads: AgentChatReadService::new(ledger.clone()),
            dispatches: ledger,
            host_epoch,
            models: Some(models),
            transport,
            cancellations: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl std::fmt::Debug for StandaloneReadiness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StandaloneReadiness(..)")
    }
}

pub(super) trait StandalonePromptRelease: Send + Sync + std::fmt::Debug {
    fn provider(&self, prompt: &PromptWake) -> Result<AgentChatProvider, String>;
    fn provision_claurst(
        &self,
        prompt: PromptWake,
        notify: std::sync::Arc<Notify>,
    ) -> Result<(), String>;
    fn release(&self, prompt: &PromptWake) -> Result<StandalonePromptReleaseOutcome, String>;
    fn fail(&self, prompt: &PromptWake, reason: &str) -> Result<(), String>;
    fn cancel_claurst_provision(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StandalonePromptReleaseOutcome {
    Routed,
    Claurst,
}

impl StandalonePromptRelease for StandaloneReadiness {
    fn provider(&self, prompt: &PromptWake) -> Result<AgentChatProvider, String> {
        if prompt.disposition != AgentChatPromptDisposition::Send {
            return Ok(AgentChatProvider::Claude);
        }
        self.reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map(|selection| selection.provider)
            .map_err(|error| error.to_string())
    }

    fn provision_claurst(
        &self,
        prompt: PromptWake,
        notify: std::sync::Arc<Notify>,
    ) -> Result<(), String> {
        let model_id = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|error| error.to_string())?
            .model;
        let models = self
            .models
            .clone()
            .ok_or_else(|| "Claurst local model authority is unavailable".to_owned())?;
        let readiness = self.clone();
        let request_id = prompt.receipt_id.0.clone();
        let transport = Arc::clone(&self.transport);
        let (cancel, mut canceled) = oneshot::channel();
        self.cancellations
            .lock()
            .map_err(|_| "local model cancellation registry is unavailable".to_owned())?
            .insert(prompt.run_id.0.clone(), cancel);
        let cancellation = ProvisionCancellation {
            values: Arc::clone(&self.cancellations),
            run_id: prompt.run_id.0.clone(),
        };
        tokio::spawn(async move {
            let _cancellation = cancellation;
            let inspected_models = models.clone();
            let inspected_model_id = model_id.clone();
            let start = tokio::task::spawn_blocking(move || {
                inspected_models.begin_download(&inspected_model_id)
            })
            .await
            .map_err(|error| error.to_string());
            let start = match start {
                Ok(start) => start,
                Err(error) => {
                    let reason = crate::local_model_events::failure_for(&error);
                    let _ = readiness.fail_download(&prompt, request_id, model_id, reason);
                    return;
                }
            };
            match start {
            Ok(crate::standalone_authority_composition::LocalModelDownloadStart::Ready { size_bytes }) => {
                let accepted = readiness.publish_local_model_frame(LocalModelFrame::DownloadAccepted {
                    request_id: request_id.clone(),
                    model_id: model_id.clone(),
                    state: LocalModelInstallState::Ready { size_bytes },
                });
                if let Err(error) = accepted {
                    let reason = crate::local_model_events::failure_for(&error);
                    let _ = readiness.fail_download(&prompt, request_id, model_id, reason);
                    return;
                }
                if let Err(error) = readiness.publish_local_model_frame(LocalModelFrame::DownloadComplete {
                    request_id: request_id.clone(),
                    model_id: model_id.clone(),
                    size_bytes,
                }) {
                    let reason = crate::local_model_events::failure_for(&error);
                    let _ = readiness.fail_prompt(
                        &prompt,
                        crate::local_model_events::failure_text(reason),
                    );
                    return;
                }
                match readiness.release(&prompt) {
                    Ok(_) => notify.notify_one(),
                    Err(error) => {
                        let reason = crate::local_model_events::failure_for(&error);
                        let _ = readiness.fail_prompt(
                            &prompt,
                            crate::local_model_events::failure_text(reason),
                        );
                    }
                }
            }
            Ok(crate::standalone_authority_composition::LocalModelDownloadStart::Download { plan, resumed_bytes }) => {
                if let Err(error) = readiness.publish_local_model_frame(LocalModelFrame::DownloadAccepted {
                    request_id: request_id.clone(),
                    model_id: model_id.clone(),
                    state: LocalModelInstallState::Downloading {
                        downloaded_bytes: resumed_bytes,
                        total_bytes: plan.expected_bytes,
                    },
                }) {
                    models.finish_download(&model_id);
                    let reason = crate::local_model_events::failure_for(&error);
                    let _ = readiness.fail_download(&prompt, request_id, model_id, reason);
                    return;
                }
                let events = readiness.clone();
                let failure_prompt = prompt.clone();
                let progress_request_id = request_id.clone();
                let progress_model_id = model_id.clone();
                let download = crate::local_model_download::download_model(&plan, transport.as_ref(), move |event| {
                        let (downloaded_bytes, total_bytes) = match event {
                            crate::local_model_download::ModelDownloadProgress::Started { downloaded_bytes, total_bytes }
                            | crate::local_model_download::ModelDownloadProgress::Advanced { downloaded_bytes, total_bytes } => {
                                (downloaded_bytes, total_bytes)
                            }
                            crate::local_model_download::ModelDownloadProgress::Complete { .. } => return,
                        };
                        let _ = events.publish_local_model_frame(LocalModelFrame::DownloadProgress {
                            request_id: progress_request_id.clone(),
                            model_id: progress_model_id.clone(),
                            downloaded_bytes,
                            total_bytes,
                        });
                });
                let outcome = tokio::select! {
                    output = download => Some(output),
                    _ = &mut canceled => None,
                };
                models.finish_download(&model_id);
                match outcome {
                    Some(Ok(_)) => {
                            let completed = readiness.publish_local_model_frame(LocalModelFrame::DownloadComplete {
                                request_id: request_id.clone(),
                                model_id: model_id.clone(),
                                size_bytes: plan.expected_bytes,
                            });
                            if let Err(error) = completed {
                                let reason = crate::local_model_events::failure_for(&error);
                                let _ = readiness.fail_prompt(
                                    &failure_prompt,
                                    crate::local_model_events::failure_text(reason),
                                );
                                return;
                            }
                            match readiness.release(&failure_prompt) {
                                Ok(_) => notify.notify_one(),
                                Err(error) => {
                                    let reason = crate::local_model_events::failure_for(&error);
                                    let _ = readiness.fail_prompt(
                                        &failure_prompt,
                                        crate::local_model_events::failure_text(reason),
                                    );
                                }
                            }
                    }
                    Some(Err(error)) => {
                            let reason =
                                crate::local_model_events::failure_for(&error.to_string());
                            let _ = readiness.fail_download(
                                &failure_prompt,
                                request_id,
                                model_id,
                                reason,
                            );
                    }
                    None => {
                        let _ = readiness.fail_download(
                            &failure_prompt,
                            request_id,
                            model_id,
                            gent_protocol::LocalModelDownloadFailure::Cancelled,
                        );
                    }
                }
            }
            Err(crate::standalone_authority_composition::StandaloneAuthorityError::ClaurstDownloadInProgress) => {
                let failure_prompt = prompt.clone();
                tokio::spawn(async move {
                    let mut reported = None;
                    loop {
                        match models.install_state(&model_id) {
                            Ok(gent_protocol::LocalModelInstallState::Ready { size_bytes }) => {
                                if reported.is_none() {
                                    let accepted = readiness.publish_local_model_frame(LocalModelFrame::DownloadAccepted {
                                        request_id: request_id.clone(),
                                        model_id: model_id.clone(),
                                        state: LocalModelInstallState::Ready { size_bytes },
                                    });
                                    if let Err(error) = accepted {
                                        let reason = crate::local_model_events::failure_for(&error);
                                        let _ = readiness.fail_download(
                                            &failure_prompt,
                                            request_id,
                                            model_id,
                                            reason,
                                        );
                                        return;
                                    }
                                }
                                let completed = readiness.publish_local_model_frame(LocalModelFrame::DownloadComplete {
                                    request_id: request_id.clone(),
                                    model_id: model_id.clone(),
                                    size_bytes,
                                });
                                if let Err(error) = completed {
                                    let reason = crate::local_model_events::failure_for(&error);
                                    let _ = readiness.fail_prompt(
                                        &failure_prompt,
                                        crate::local_model_events::failure_text(reason),
                                    );
                                    return;
                                }
                                match readiness.release(&prompt) {
                                    Ok(_) => notify.notify_one(),
                                    Err(error) => {
                                        let reason = crate::local_model_events::failure_for(&error);
                                        let _ = readiness.fail_prompt(
                                            &failure_prompt,
                                            crate::local_model_events::failure_text(reason),
                                        );
                                    }
                                }
                                return;
                            }
                            Ok(gent_protocol::LocalModelInstallState::Downloading {
                                downloaded_bytes,
                                total_bytes,
                            }) if models.download_active(&model_id) => {
                                let current = (downloaded_bytes, total_bytes);
                                if reported != Some(current) {
                                    let frame = if reported.is_none() {
                                        LocalModelFrame::DownloadAccepted {
                                            request_id: request_id.clone(),
                                            model_id: model_id.clone(),
                                            state: LocalModelInstallState::Downloading {
                                                downloaded_bytes,
                                                total_bytes,
                                            },
                                        }
                                    } else {
                                        LocalModelFrame::DownloadProgress {
                                            request_id: request_id.clone(),
                                            model_id: model_id.clone(),
                                            downloaded_bytes,
                                            total_bytes,
                                        }
                                    };
                                    if let Err(error) = readiness.publish_local_model_frame(frame) {
                                        let reason = crate::local_model_events::failure_for(&error);
                                        let _ = readiness.fail_download(
                                            &failure_prompt,
                                            request_id,
                                            model_id,
                                            reason,
                                        );
                                        return;
                                    }
                                    reported = Some(current);
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            Ok(_) => {
                                let _ = readiness.fail_download(
                                    &failure_prompt,
                                    request_id,
                                    model_id,
                                    gent_protocol::LocalModelDownloadFailure::TransportFailed,
                                );
                                return;
                            }
                            Err(error) => {
                                let reason =
                                    crate::local_model_events::failure_for(&error.to_string());
                                let _ = readiness.fail_download(
                                    &failure_prompt,
                                    request_id,
                                    model_id,
                                    reason,
                                );
                                return;
                            }
                        }
                    }
                });
            }
            Err(error) => {
                let reason = crate::local_model_events::failure_for(&error.to_string());
                let _ = readiness.fail_download(&prompt, request_id, model_id, reason);
            }
        }
        });
        Ok(())
    }

    fn cancel_claurst_provision(&self, run_id: &str) -> Result<bool, String> {
        let sender = self
            .cancellations
            .lock()
            .map_err(|_| "local model cancellation registry is unavailable".to_owned())?
            .remove(run_id);
        Ok(sender.is_some_and(|sender| sender.send(()).is_ok()))
    }

    fn release(&self, prompt: &PromptWake) -> Result<StandalonePromptReleaseOutcome, String> {
        if prompt.disposition != AgentChatPromptDisposition::Send {
            return Ok(StandalonePromptReleaseOutcome::Routed);
        }
        let selection = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|error| error.to_string())?;
        if !matches!(
            selection.provider,
            AgentChatProvider::Claude | AgentChatProvider::Codex | AgentChatProvider::Claurst
        ) {
            return Err("standalone authority does not enable the selected provider".into());
        }
        let binding = ProviderPromptReadinessBinding {
            prompt_receipt_id: prompt.receipt_id.clone(),
            conversation_id: prompt.conversation_id.clone(),
            run_id: prompt.run_id.clone(),
            provider: selection.provider,
        };
        let (command, terminal) =
            crate::prompt_readiness_admission::decision(&binding, self.host_epoch)?;
        self.dispatches
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .map_err(|error| error.to_string())
            .map(|_| match selection.provider {
                AgentChatProvider::Claurst => StandalonePromptReleaseOutcome::Claurst,
                AgentChatProvider::Claude | AgentChatProvider::Codex => {
                    StandalonePromptReleaseOutcome::Routed
                }
            })
    }

    fn fail(&self, prompt: &PromptWake, reason: &str) -> Result<(), String> {
        self.fail_prompt(prompt, reason)
    }
}

impl StandaloneReadiness {
    fn publish_local_model_frame(&self, frame: LocalModelFrame) -> Result<(), String> {
        crate::local_model_events::publish(&self.dispatches, self.host_epoch, frame)
    }

    fn fail_download(
        &self,
        prompt: &PromptWake,
        request_id: String,
        model_id: String,
        reason: gent_protocol::LocalModelDownloadFailure,
    ) -> Result<(), String> {
        let reason_text = crate::local_model_events::failure_text(reason);
        let settlement_result = self.fail_prompt(prompt, reason_text);
        let frame_result = self.publish_local_model_frame(LocalModelFrame::DownloadFailed {
            request_id,
            model_id,
            reason,
        });
        frame_result.and(settlement_result)
    }

    fn fail_prompt(&self, prompt: &PromptWake, reason: &str) -> Result<(), String> {
        let provider = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|error| error.to_string())?
            .provider;
        let binding = ProviderPromptReadinessFailureBinding {
            prompt_receipt_id: prompt.receipt_id.clone(),
            conversation_id: prompt.conversation_id.clone(),
            run_id: prompt.run_id.clone(),
            provider,
            reason: reason.to_owned(),
        };
        let (command, terminal) =
            crate::prompt_readiness_admission::failure(&binding, self.host_epoch)?;
        self.dispatches
            .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use gent_ports::{
        AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationLedger, Ledger,
        TranscriptLedger,
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
        standalone_authority_composition::StandaloneClaurstModels,
    };

    #[derive(Debug)]
    struct Transport;

    #[derive(Debug)]
    struct Response(Option<Vec<u8>>);

    #[derive(Debug)]
    struct BlockingTransport;

    #[derive(Debug)]
    struct BlockingResponse;

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

    #[async_trait]
    impl ModelDownloadTransport for BlockingTransport {
        async fn get(
            &self,
            _: DownloadRequest,
        ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
            Ok(Box::new(BlockingResponse))
        }
    }

    #[async_trait]
    impl ModelDownloadResponse for BlockingResponse {
        fn status(&self) -> u16 {
            200
        }

        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
            std::future::pending().await
        }
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
        let readiness = StandaloneReadiness::with_transport(
            ledger.clone(),
            HostEpoch(1),
            StandaloneClaurstModels::from_data_dir(directory.path()).unwrap(),
            Arc::new(Transport),
        );

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
                serde_json::from_value::<gent_protocol::LocalModelFrame>(event.payload.clone())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(frames.iter().all(|frame| match frame {
            gent_protocol::LocalModelFrame::DownloadAccepted { request_id, .. }
            | gent_protocol::LocalModelFrame::DownloadProgress { request_id, .. }
            | gent_protocol::LocalModelFrame::DownloadComplete { request_id, .. }
            | gent_protocol::LocalModelFrame::DownloadFailed { request_id, .. } => {
                request_id == "prompt"
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
    async fn canceling_claurst_download_fails_the_prompt_without_leaving_an_active_download() {
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
        let models = StandaloneClaurstModels::from_data_dir(directory.path()).unwrap();
        let readiness = StandaloneReadiness::with_transport(
            ledger.clone(),
            HostEpoch(1),
            models.clone(),
            Arc::new(BlockingTransport),
        );

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
                        serde_json::from_value::<gent_protocol::LocalModelFrame>(event.payload)
                            .unwrap()
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
        assert!(readiness.cancel_claurst_provision("run").unwrap());

        let frames = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let frames = ledger
                    .read_event_page(0, 10)
                    .unwrap()
                    .events
                    .into_iter()
                    .filter(|event| event.kind == "localModelDownload")
                    .map(|event| {
                        serde_json::from_value::<gent_protocol::LocalModelFrame>(event.payload)
                            .unwrap()
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
        assert!(!models.download_active("qwen3-8b-q4-k-m"));
        assert_eq!(
            ledger
                .find_turn(&saved.message.turn_id)
                .unwrap()
                .unwrap()
                .phase,
            gent_types::DurableTurnPhase::Failed
        );
    }
}
