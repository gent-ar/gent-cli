//! Resumable HTTP download execution for a previously approved local-model plan.

use crate::local_model_provisioning::LocalModelDownloadPlan;
use async_trait::async_trait;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadRequest {
    pub(crate) url: String,
    pub(crate) range_start: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelDownloadProgress {
    Started {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Advanced {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Complete {
        path: PathBuf,
        total_bytes: u64,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ModelDownloadError {
    #[error("download destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("download partial file is not a regular file: {0}")]
    UnsafePartial(PathBuf),
    #[error("download partial has {actual_bytes} bytes but expected at most {expected_bytes}")]
    PartialTooLarge {
        actual_bytes: u64,
        expected_bytes: u64,
    },
    #[error("server refused byte-range resume with HTTP {0}")]
    ResumeRejected(u16),
    #[error("unexpected download HTTP status {0}")]
    UnexpectedStatus(u16),
    #[error("download exceeded its approved size of {expected_bytes} bytes")]
    ExceededApprovedSize { expected_bytes: u64 },
    #[error("download finished at {actual_bytes} bytes but expected {expected_bytes}")]
    SizeMismatch {
        actual_bytes: u64,
        expected_bytes: u64,
    },
    #[error("download transport failed: {0}")]
    Transport(String),
    #[error("local model download I/O failed: {0}")]
    Io(String),
}

#[async_trait]
pub(crate) trait ModelDownloadTransport: std::fmt::Debug + Send + Sync {
    async fn get(
        &self,
        request: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError>;
}

#[async_trait]
pub(crate) trait ModelDownloadResponse: std::fmt::Debug + Send {
    fn status(&self) -> u16;
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError>;
}

#[derive(Clone, Debug)]
pub(crate) struct ReqwestModelDownloadTransport {
    client: reqwest::Client,
}

impl ReqwestModelDownloadTransport {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ModelDownloadTransport for ReqwestModelDownloadTransport {
    async fn get(
        &self,
        request: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        let mut request_builder = self.client.get(request.url);
        if let Some(start) = request.range_start {
            request_builder =
                request_builder.header(reqwest::header::RANGE, format!("bytes={start}-"));
        }
        let response = request_builder
            .send()
            .await
            .map_err(|error| ModelDownloadError::Transport(error.to_string()))?;
        Ok(Box::new(ReqwestModelDownloadResponse { response }))
    }
}

#[derive(Debug)]
struct ReqwestModelDownloadResponse {
    response: reqwest::Response,
}

#[async_trait]
impl ModelDownloadResponse for ReqwestModelDownloadResponse {
    fn status(&self) -> u16 {
        self.response.status().as_u16()
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        self.response
            .chunk()
            .await
            .map(|chunk| chunk.map(|chunk| chunk.to_vec()))
            .map_err(|error| ModelDownloadError::Transport(error.to_string()))
    }
}

pub(crate) async fn download_model(
    plan: &LocalModelDownloadPlan,
    transport: &dyn ModelDownloadTransport,
    mut report: impl FnMut(ModelDownloadProgress),
) -> Result<PathBuf, ModelDownloadError> {
    if plan.destination.exists() {
        return Err(ModelDownloadError::DestinationExists(
            plan.destination.clone(),
        ));
    }
    let resumed_bytes = partial_size(&plan.partial_destination)?;
    if resumed_bytes > plan.expected_bytes {
        return Err(ModelDownloadError::PartialTooLarge {
            actual_bytes: resumed_bytes,
            expected_bytes: plan.expected_bytes,
        });
    }
    let parent = plan
        .partial_destination
        .parent()
        .ok_or_else(|| ModelDownloadError::Io("partial destination has no parent".into()))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let mut response = transport
        .get(DownloadRequest {
            url: plan.source_url.clone(),
            range_start: (resumed_bytes != 0).then_some(resumed_bytes),
        })
        .await?;
    validate_status(response.status(), resumed_bytes)?;
    report(ModelDownloadProgress::Started {
        downloaded_bytes: resumed_bytes,
        total_bytes: plan.expected_bytes,
    });
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&plan.partial_destination)
        .map_err(io_error)?;
    let mut downloaded_bytes = resumed_bytes;
    while let Some(chunk) = response.next_chunk().await? {
        downloaded_bytes = downloaded_bytes.checked_add(chunk.len() as u64).ok_or(
            ModelDownloadError::ExceededApprovedSize {
                expected_bytes: plan.expected_bytes,
            },
        )?;
        if downloaded_bytes > plan.expected_bytes {
            return Err(ModelDownloadError::ExceededApprovedSize {
                expected_bytes: plan.expected_bytes,
            });
        }
        output.write_all(&chunk).map_err(io_error)?;
        report(ModelDownloadProgress::Advanced {
            downloaded_bytes,
            total_bytes: plan.expected_bytes,
        });
    }
    output.sync_all().map_err(io_error)?;
    if downloaded_bytes != plan.expected_bytes {
        return Err(ModelDownloadError::SizeMismatch {
            actual_bytes: downloaded_bytes,
            expected_bytes: plan.expected_bytes,
        });
    }
    fs::rename(&plan.partial_destination, &plan.destination).map_err(io_error)?;
    report(ModelDownloadProgress::Complete {
        path: plan.destination.clone(),
        total_bytes: plan.expected_bytes,
    });
    Ok(plan.destination.clone())
}

fn validate_status(status: u16, resumed_bytes: u64) -> Result<(), ModelDownloadError> {
    if resumed_bytes != 0 {
        return if status == 206 {
            Ok(())
        } else {
            Err(ModelDownloadError::ResumeRejected(status))
        };
    }
    if status == 200 {
        Ok(())
    } else {
        Err(ModelDownloadError::UnexpectedStatus(status))
    }
}

fn partial_size(path: &Path) -> Result<u64, ModelDownloadError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(ModelDownloadError::UnsafePartial(path.to_path_buf()))
        }
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(io_error(error)),
    }
}

fn io_error(error: std::io::Error) -> ModelDownloadError {
    ModelDownloadError::Io(error.to_string())
}
