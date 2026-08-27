use std::sync::Arc;

use gent_protocol::{
    LOCAL_MODELS_CAPABILITY, LocalModelDownloadFailure, LocalModelFrame, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::{io::AsyncWrite, sync::mpsc};

use crate::{
    api::RuntimeApi,
    local_model_download::{
        ModelDownloadProgress, ModelDownloadTransport, ReqwestModelDownloadTransport,
        download_model,
    },
    standalone_authority_composition::LocalModelDownloadStart,
    transport::write_error,
};

pub(crate) trait LocalModelPort {
    fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String>;
    fn begin_local_model_download(&self, model_id: &str)
    -> Result<LocalModelDownloadStart, String>;
    fn finish_local_model_download(&self, model_id: &str);
    fn publish_local_model_frame(&self, frame: LocalModelFrame) -> Result<(), String>;
}

impl<R: RuntimeApi> LocalModelPort for R {
    fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String> {
        RuntimeApi::local_models(self, frame)
    }

    fn begin_local_model_download(
        &self,
        model_id: &str,
    ) -> Result<LocalModelDownloadStart, String> {
        RuntimeApi::begin_local_model_download(self, model_id)
    }

    fn finish_local_model_download(&self, model_id: &str) {
        RuntimeApi::finish_local_model_download(self, model_id);
    }

    fn publish_local_model_frame(&self, frame: LocalModelFrame) -> Result<(), String> {
        RuntimeApi::publish_local_model_frame(self, frame)
    }
}

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    dispatch_port(
        stream,
        runtime,
        capabilities,
        raw,
        Arc::new(ReqwestModelDownloadTransport::new()),
    )
    .await
}

async fn dispatch_port<S, P>(
    stream: &mut S,
    port: &P,
    capabilities: &CapabilitySet,
    raw: &Value,
    transport: Arc<dyn ModelDownloadTransport>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    P: LocalModelPort,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == LOCAL_MODELS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<LocalModelFrame>(raw.clone()) else {
        return Ok(false);
    };
    if let Err(error) = frame.validate() {
        write_error(stream, "invalidLocalModel", &error.to_string()).await?;
        return Ok(true);
    }
    match frame.clone() {
        LocalModelFrame::Download {
            request_id,
            model_id,
        } => {
            dispatch_download(stream, port, request_id, model_id, transport).await?;
        }
        LocalModelFrame::List { .. } | LocalModelFrame::Status { .. } => {
            match port.local_models(frame.clone()) {
                Ok(reply) if correlated(&frame, &reply) => write_json_frame(stream, &reply).await?,
                Ok(_) => {
                    write_error(
                        stream,
                        "invalidLocalModel",
                        "local-model runtime returned an uncorrelated response",
                    )
                    .await?;
                }
                Err(message) => write_error(stream, "localModelUnavailable", &message).await?,
            }
        }
        _ => {
            write_error(
                stream,
                "invalidLocalModel",
                "local-model response frames are server-only",
            )
            .await?;
        }
    }
    Ok(true)
}

async fn dispatch_download<S, P>(
    stream: &mut S,
    port: &P,
    request_id: String,
    model_id: String,
    transport: Arc<dyn ModelDownloadTransport>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    P: LocalModelPort,
{
    let start = match port.begin_local_model_download(&model_id) {
        Ok(start) => start,
        Err(error) => {
            let frame = LocalModelFrame::DownloadFailed {
                request_id,
                model_id,
                reason: crate::local_model_events::failure_for(&error),
            };
            port.publish_local_model_frame(frame.clone())?;
            write_json_frame(stream, &frame).await?;
            return Ok(());
        }
    };
    match start {
        LocalModelDownloadStart::Ready { size_bytes } => {
            let accepted = LocalModelFrame::DownloadAccepted {
                request_id: request_id.clone(),
                model_id: model_id.clone(),
                state: gent_protocol::LocalModelInstallState::Ready { size_bytes },
            };
            port.publish_local_model_frame(accepted.clone())?;
            write_json_frame(stream, &accepted).await?;
            let complete = LocalModelFrame::DownloadComplete {
                request_id,
                model_id,
                size_bytes,
            };
            port.publish_local_model_frame(complete.clone())?;
            write_json_frame(stream, &complete).await?;
        }
        LocalModelDownloadStart::Download {
            plan,
            resumed_bytes,
        } => {
            let accepted = LocalModelFrame::DownloadAccepted {
                request_id: request_id.clone(),
                model_id: model_id.clone(),
                state: gent_protocol::LocalModelInstallState::Downloading {
                    downloaded_bytes: resumed_bytes,
                    total_bytes: plan.expected_bytes,
                },
            };
            if let Err(error) = port.publish_local_model_frame(accepted.clone()) {
                port.finish_local_model_download(&model_id);
                return Err(error.into());
            }
            if let Err(error) = write_json_frame(stream, &accepted).await {
                port.finish_local_model_download(&model_id);
                return Err(error.into());
            }
            let (events, mut receive) = mpsc::unbounded_channel();
            let expected_bytes = plan.expected_bytes;
            let task = tokio::spawn(async move {
                download_model(&plan, transport.as_ref(), |event| {
                    let _ = events.send(event);
                })
                .await
            });
            let mut stream_error = None;
            while let Some(event) = receive.recv().await {
                match event {
                    ModelDownloadProgress::Started {
                        downloaded_bytes,
                        total_bytes,
                    }
                    | ModelDownloadProgress::Advanced {
                        downloaded_bytes,
                        total_bytes,
                    } => {
                        let progress = LocalModelFrame::DownloadProgress {
                            request_id: request_id.clone(),
                            model_id: model_id.clone(),
                            downloaded_bytes,
                            total_bytes,
                        };
                        if let Err(error) = port.publish_local_model_frame(progress.clone()) {
                            stream_error = Some(error.into());
                            break;
                        }
                        if let Err(error) = write_json_frame(stream, &progress).await {
                            stream_error = Some(error.into());
                            break;
                        }
                    }
                    ModelDownloadProgress::Complete { total_bytes, .. } => {
                        debug_assert_eq!(total_bytes, expected_bytes);
                    }
                }
            }
            if let Some(error) = stream_error {
                task.abort();
                let _ = task.await;
                port.finish_local_model_download(&model_id);
                return Err(error);
            }
            let outcome = task.await;
            port.finish_local_model_download(&model_id);
            match outcome {
                Ok(Ok(_)) => {
                    let complete = LocalModelFrame::DownloadComplete {
                        request_id,
                        model_id,
                        size_bytes: expected_bytes,
                    };
                    port.publish_local_model_frame(complete.clone())?;
                    write_json_frame(stream, &complete).await?;
                }
                Ok(Err(error)) => {
                    let failed = LocalModelFrame::DownloadFailed {
                        request_id,
                        model_id,
                        reason: crate::local_model_events::failure_for(&error.to_string()),
                    };
                    port.publish_local_model_frame(failed.clone())?;
                    write_json_frame(stream, &failed).await?;
                }
                Err(_) => {
                    let failed = LocalModelFrame::DownloadFailed {
                        request_id,
                        model_id,
                        reason: LocalModelDownloadFailure::TransportFailed,
                    };
                    port.publish_local_model_frame(failed.clone())?;
                    write_json_frame(stream, &failed).await?;
                }
            }
        }
    }
    Ok(())
}

fn correlated(request: &LocalModelFrame, reply: &LocalModelFrame) -> bool {
    match (request, reply) {
        (
            LocalModelFrame::List { request_id },
            LocalModelFrame::Catalogue {
                request_id: actual, ..
            },
        ) => actual == request_id,
        (
            LocalModelFrame::Status {
                request_id,
                model_id,
            },
            LocalModelFrame::StatusResult {
                request_id: actual_request,
                model_id: actual_model,
                ..
            },
        ) => actual_request == request_id && actual_model == model_id,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use gent_protocol::{LocalModelDescriptor, LocalModelInstallState, read_json_frame};
    use serde_json::json;
    use tokio::io::duplex;

    use super::{LocalModelPort, dispatch_port};
    use crate::{
        local_model_download::{
            DownloadRequest, ModelDownloadError, ModelDownloadResponse, ModelDownloadTransport,
        },
        local_model_provisioning::LocalModelDownloadPlan,
        standalone_authority_composition::LocalModelDownloadStart,
    };
    use gent_protocol::{LOCAL_MODELS_CAPABILITY, LocalModelFrame};
    use gent_types::CapabilitySet;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::io::AsyncWrite;

    #[derive(Clone)]
    struct Catalogue {
        plan: LocalModelDownloadPlan,
        completed: Arc<Mutex<Vec<String>>>,
        events: Arc<Mutex<Vec<LocalModelFrame>>>,
    }

    impl LocalModelPort for Catalogue {
        fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String> {
            match frame {
                LocalModelFrame::List { request_id } => Ok(LocalModelFrame::Catalogue {
                    request_id,
                    models: vec![LocalModelDescriptor {
                        id: "model".into(),
                        label: "Model".into(),
                        size_bytes: 10,
                    }],
                }),
                LocalModelFrame::Status {
                    request_id,
                    model_id,
                } => Ok(LocalModelFrame::StatusResult {
                    request_id,
                    model_id,
                    state: LocalModelInstallState::NotInstalled,
                }),
                _ => Err("unexpected".into()),
            }
        }

        fn begin_local_model_download(
            &self,
            model_id: &str,
        ) -> Result<LocalModelDownloadStart, String> {
            if model_id == "missing" {
                return Err("not in Gent's curated catalog".into());
            }
            Ok(LocalModelDownloadStart::Download {
                plan: self.plan.clone(),
                resumed_bytes: 0,
            })
        }

        fn finish_local_model_download(&self, model_id: &str) {
            self.completed.lock().unwrap().push(model_id.into());
        }

        fn publish_local_model_frame(&self, frame: LocalModelFrame) -> Result<(), String> {
            self.events.lock().unwrap().push(frame);
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeTransport;

    #[derive(Debug)]
    struct FakeResponse {
        chunks: Vec<Vec<u8>>,
    }

    #[async_trait]
    impl ModelDownloadTransport for FakeTransport {
        async fn get(
            &self,
            request: DownloadRequest,
        ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
            assert_eq!(request.range_start, None);
            assert_eq!(request.url, "https://huggingface.co/gent/model.gguf");
            Ok(Box::new(FakeResponse {
                chunks: vec![vec![1, 2, 3], vec![4; 7]],
            }))
        }
    }

    #[async_trait]
    impl ModelDownloadResponse for FakeResponse {
        fn status(&self) -> u16 {
            200
        }
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
            Ok((!self.chunks.is_empty()).then(|| self.chunks.remove(0)))
        }
    }

    struct FailingWriter;

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "client closed",
            )))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn port() -> (tempfile::TempDir, Catalogue) {
        let directory = tempfile::tempdir().unwrap();
        let plan = LocalModelDownloadPlan {
            model_id: "model".into(),
            source_url: "https://huggingface.co/gent/model.gguf".into(),
            expected_bytes: 10,
            expected_sha256: "9f13fcad050f30bc4430cc166057e7992f01d82d85b607a923ae3d7c6a7688ee"
                .into(),
            destination: directory.path().join("models/model.gguf"),
            partial_destination: directory.path().join("models/model.gguf.part"),
        };
        (
            directory,
            Catalogue {
                plan,
                completed: Arc::new(Mutex::new(Vec::new())),
                events: Arc::new(Mutex::new(Vec::new())),
            },
        )
    }

    #[tokio::test]
    async fn consented_curated_download_streams_progress_and_terminal_completion() {
        let (_directory, port) = port();
        let capability = CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]);
        let request =
            json!({"type":"download","body":{"requestId":"download-1","modelId":"model"}});
        let (mut client, mut server) = duplex(4096);
        let completed = Arc::clone(&port.completed);
        let events = Arc::clone(&port.events);
        let task = tokio::spawn(async move {
            dispatch_port(
                &mut server,
                &port,
                &capability,
                &request,
                Arc::new(FakeTransport),
            )
            .await
            .unwrap();
        });
        let mut frames = Vec::new();
        for _ in 0..4 {
            frames.push(
                read_json_frame::<_, LocalModelFrame>(&mut client)
                    .await
                    .unwrap(),
            );
        }
        task.await.unwrap();
        assert!(
            matches!(frames[0], LocalModelFrame::DownloadAccepted { ref request_id, ref model_id, state: LocalModelInstallState::Downloading { downloaded_bytes: 0, total_bytes: 10 } } if request_id == "download-1" && model_id == "model")
        );
        assert!(matches!(
            frames[1],
            LocalModelFrame::DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: 10,
                ..
            }
        ));
        assert!(matches!(
            frames[2],
            LocalModelFrame::DownloadProgress {
                downloaded_bytes: 10,
                total_bytes: 10,
                ..
            }
        ));
        assert!(matches!(
            frames[3],
            LocalModelFrame::DownloadComplete { size_bytes: 10, .. }
        ));
        assert_eq!(*completed.lock().unwrap(), vec!["model"]);
        assert_eq!(*events.lock().unwrap(), frames);
    }

    #[tokio::test]
    async fn unknown_model_is_a_typed_terminal_failure_without_starting_a_transfer() {
        let (_directory, port) = port();
        let capability = CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]);
        let request =
            json!({"type":"download","body":{"requestId":"download-1","modelId":"missing"}});
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move {
            dispatch_port(
                &mut server,
                &port,
                &capability,
                &request,
                Arc::new(FakeTransport),
            )
            .await
            .unwrap();
        });
        assert!(matches!(
            read_json_frame::<_, LocalModelFrame>(&mut client)
                .await
                .unwrap(),
            LocalModelFrame::DownloadFailed {
                reason: gent_protocol::LocalModelDownloadFailure::UnknownModel,
                ..
            }
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn closed_client_releases_the_download_claim() {
        let (_directory, port) = port();
        let capability = CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]);
        let request =
            json!({"type":"download","body":{"requestId":"download-1","modelId":"model"}});
        let mut writer = FailingWriter;
        let result = dispatch_port(
            &mut writer,
            &port,
            &capability,
            &request,
            Arc::new(FakeTransport),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(*port.completed.lock().unwrap(), vec!["model"]);
    }
}
