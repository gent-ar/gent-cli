//! Resumable HTTP download execution for a previously approved local-model plan.

use crate::{
    local_model_integrity::{file_digest, remember_sha256},
    local_model_provisioning::LocalModelDownloadPlan,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const PROGRESS_INCREMENT_BYTES: u64 = 1_048_576;
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_RETRIES: u32 = 2;

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
    #[error("download does not match the curated SHA-256")]
    DigestMismatch,
    #[error("download stalled: no data arrived for {0} seconds")]
    Stalled(u64),
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

#[path = "local_model_download_http.rs"]
mod http;
pub(crate) use http::ReqwestModelDownloadTransport;

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
    let mut received = Received {
        bytes: resumed_bytes,
        reported_bytes: resumed_bytes,
        digest: if resumed_bytes == 0 {
            Sha256::new()
        } else {
            file_digest(&plan.partial_destination).map_err(io_error)?
        },
    };
    let mut stalls = 0;
    while received.bytes < plan.expected_bytes {
        match receive(plan, transport, &mut received, &mut report).await {
            Err(ModelDownloadError::Stalled(_)) if stalls < STALL_RETRIES => stalls += 1,
            result => {
                result?;
                break;
            }
        }
    }
    if received.bytes != plan.expected_bytes {
        return Err(ModelDownloadError::SizeMismatch {
            actual_bytes: received.bytes,
            expected_bytes: plan.expected_bytes,
        });
    }
    if hex::encode(received.digest.finalize()) != plan.expected_sha256 {
        remove_partial(&plan.partial_destination)?;
        return Err(ModelDownloadError::DigestMismatch);
    }
    publish(plan, &mut report)
}

struct Received {
    bytes: u64,
    reported_bytes: u64,
    digest: Sha256,
}

async fn receive(
    plan: &LocalModelDownloadPlan,
    transport: &dyn ModelDownloadTransport,
    received: &mut Received,
    report: &mut impl FnMut(ModelDownloadProgress),
) -> Result<(), ModelDownloadError> {
    let mut response = within_stall_timeout(transport.get(DownloadRequest {
        url: plan.source_url.clone(),
        range_start: (received.bytes != 0).then_some(received.bytes),
    }))
    .await?;
    validate_status(response.status(), received.bytes)?;
    report(ModelDownloadProgress::Started {
        downloaded_bytes: received.bytes,
        total_bytes: plan.expected_bytes,
    });
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&plan.partial_destination)
        .map_err(io_error)?;
    while let Some(chunk) = within_stall_timeout(response.next_chunk()).await? {
        let downloaded_bytes = received
            .bytes
            .checked_add(chunk.len() as u64)
            .filter(|bytes| *bytes <= plan.expected_bytes)
            .ok_or(ModelDownloadError::ExceededApprovedSize {
                expected_bytes: plan.expected_bytes,
            })?;
        output.write_all(&chunk).map_err(io_error)?;
        received.digest.update(&chunk);
        received.bytes = downloaded_bytes;
        if downloaded_bytes == plan.expected_bytes
            || downloaded_bytes.saturating_sub(received.reported_bytes) >= PROGRESS_INCREMENT_BYTES
        {
            if plan.destination.exists() {
                drop(output);
                remove_partial(&plan.partial_destination)?;
                return Err(ModelDownloadError::DestinationExists(
                    plan.destination.clone(),
                ));
            }
            report(ModelDownloadProgress::Advanced {
                downloaded_bytes,
                total_bytes: plan.expected_bytes,
            });
            received.reported_bytes = downloaded_bytes;
        }
    }
    output.sync_all().map_err(io_error)
}

async fn within_stall_timeout<T>(
    operation: impl Future<Output = Result<T, ModelDownloadError>>,
) -> Result<T, ModelDownloadError> {
    tokio::time::timeout(STALL_TIMEOUT, operation)
        .await
        .map_err(|_| ModelDownloadError::Stalled(STALL_TIMEOUT.as_secs()))?
}

fn publish(
    plan: &LocalModelDownloadPlan,
    report: &mut impl FnMut(ModelDownloadProgress),
) -> Result<PathBuf, ModelDownloadError> {
    fs::rename(&plan.partial_destination, &plan.destination).map_err(io_error)?;
    remember_sha256(&plan.destination, &plan.expected_sha256);
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

fn remove_partial(path: &Path) -> Result<(), ModelDownloadError> {
    fs::remove_file(path).map_err(io_error)
}

fn io_error(error: std::io::Error) -> ModelDownloadError {
    ModelDownloadError::Io(error.to_string())
}

#[cfg(test)]
#[path = "local_model_download_tests.rs"]
mod tests;
