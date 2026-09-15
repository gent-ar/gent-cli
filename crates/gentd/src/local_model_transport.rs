use gent_protocol::{
    LOCAL_MODELS_CAPABILITY, LocalModelDownloadFailure, LocalModelFrame, LocalModelInstallState,
    write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{
    api::RuntimeApi,
    local_model_jobs::{DownloadProgress, DownloadStart},
    transport::write_error,
};

pub(crate) trait LocalModelPort {
    fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String>;
    fn start_local_model_download(
        &self,
        model_id: &str,
    ) -> Result<DownloadStart, LocalModelDownloadFailure>;
}

impl<R: RuntimeApi> LocalModelPort for R {
    fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String> {
        RuntimeApi::local_models(self, frame)
    }

    fn start_local_model_download(
        &self,
        model_id: &str,
    ) -> Result<DownloadStart, LocalModelDownloadFailure> {
        RuntimeApi::start_local_model_download(self, model_id)
    }
}

pub(crate) async fn dispatch<S, P>(
    stream: &mut S,
    port: &P,
    capabilities: &CapabilitySet,
    raw: &Value,
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
            dispatch_download(stream, port, request_id, model_id).await?;
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    P: LocalModelPort,
{
    let frame = |progress: DownloadProgress| match progress {
        DownloadProgress::Downloading {
            downloaded_bytes,
            total_bytes,
        } => LocalModelFrame::DownloadProgress {
            request_id: request_id.clone(),
            model_id: model_id.clone(),
            downloaded_bytes,
            total_bytes,
        },
        DownloadProgress::Complete { size_bytes } => LocalModelFrame::DownloadComplete {
            request_id: request_id.clone(),
            model_id: model_id.clone(),
            size_bytes,
        },
        DownloadProgress::Failed(reason) => LocalModelFrame::DownloadFailed {
            request_id: request_id.clone(),
            model_id: model_id.clone(),
            reason,
        },
    };
    let mut progress = match port.start_local_model_download(&model_id) {
        Ok(DownloadStart::Downloading(progress)) => progress,
        Ok(DownloadStart::Ready { size_bytes }) => {
            let (_, progress) =
                tokio::sync::watch::channel(DownloadProgress::Complete { size_bytes });
            progress
        }
        Err(reason) => {
            write_json_frame(stream, &frame(DownloadProgress::Failed(reason))).await?;
            return Ok(());
        }
    };
    let mut current = *progress.borrow_and_update();
    let state = match current {
        DownloadProgress::Downloading {
            downloaded_bytes,
            total_bytes,
        } => LocalModelInstallState::Downloading {
            downloaded_bytes,
            total_bytes,
        },
        DownloadProgress::Complete { size_bytes } => LocalModelInstallState::Ready { size_bytes },
        DownloadProgress::Failed(_) => LocalModelInstallState::NotInstalled,
    };
    write_json_frame(
        stream,
        &LocalModelFrame::DownloadAccepted {
            request_id: request_id.clone(),
            model_id: model_id.clone(),
            state,
        },
    )
    .await?;
    while let DownloadProgress::Downloading { .. } = current {
        if progress.changed().await.is_err() {
            current = DownloadProgress::Failed(LocalModelDownloadFailure::TransportFailed);
            break;
        }
        let previous = current;
        current = *progress.borrow_and_update();
        if matches!(current, DownloadProgress::Downloading { .. }) && current != previous {
            write_json_frame(stream, &frame(current)).await?;
        }
    }
    write_json_frame(stream, &frame(current)).await?;
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
#[path = "local_model_transport_tests.rs"]
mod tests;
