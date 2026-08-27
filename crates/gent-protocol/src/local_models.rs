//! Capability-gated local GGUF model catalogue and download-progress frames.
//!
//! Both the standalone CLI and the native application use this surface. The daemon alone chooses
//! the curated model URL and Gent-owned destination; clients only name a catalogue model ID.

use serde::{Deserialize, Serialize};

/// Required before a client may browse or provision curated local models.
pub const LOCAL_MODELS_CAPABILITY: &str = "local-models-v1";

pub const DEFAULT_LOCAL_MODEL_ID: &str = "qwen3-1-7b-q4-k-m";

/// Public metadata for one model selected by Gent's curated catalogue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalModelDescriptor {
    pub id: String,
    pub label: String,
    pub size_bytes: u64,
}

/// Durable local installation state. Filesystem paths and download URLs remain daemon-private.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub enum LocalModelInstallState {
    NotInstalled,
    Downloading {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Ready {
        size_bytes: u64,
    },
}

/// Typed reason that a local-model download could not start or complete.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalModelDownloadFailure {
    UnknownModel,
    AlreadyDownloading,
    StorageUnavailable,
    TransportFailed,
    VerificationFailed,
    Cancelled,
}

/// Request/reply frames for the local model catalogue and a single download stream.
///
/// A `Download` is a subscription: after `DownloadAccepted`, the server sends zero or more
/// `DownloadProgress` frames followed by exactly one terminal `DownloadComplete` or
/// `DownloadFailed`, all with the same `request_id`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LocalModelFrame {
    List {
        request_id: String,
    },
    Catalogue {
        request_id: String,
        models: Vec<LocalModelDescriptor>,
    },
    Status {
        request_id: String,
        model_id: String,
    },
    StatusResult {
        request_id: String,
        model_id: String,
        state: LocalModelInstallState,
    },
    Download {
        request_id: String,
        model_id: String,
    },
    DownloadAccepted {
        request_id: String,
        model_id: String,
        state: LocalModelInstallState,
    },
    DownloadProgress {
        request_id: String,
        model_id: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    DownloadComplete {
        request_id: String,
        model_id: String,
        size_bytes: u64,
    },
    DownloadFailed {
        request_id: String,
        model_id: String,
        reason: LocalModelDownloadFailure,
    },
}

impl LocalModelFrame {
    /// Rejects malformed client-controlled identifiers and contradictory progress values.
    ///
    /// # Errors
    /// Returns [`LocalModelFrameError`] when a frame is not safe to route or represent.
    pub fn validate(&self) -> Result<(), LocalModelFrameError> {
        let (request_id, model_id) = match self {
            Self::List { request_id } | Self::Catalogue { request_id, .. } => (request_id, None),
            Self::Status {
                request_id,
                model_id,
            }
            | Self::StatusResult {
                request_id,
                model_id,
                ..
            }
            | Self::Download {
                request_id,
                model_id,
            }
            | Self::DownloadAccepted {
                request_id,
                model_id,
                ..
            }
            | Self::DownloadProgress {
                request_id,
                model_id,
                ..
            }
            | Self::DownloadComplete {
                request_id,
                model_id,
                ..
            }
            | Self::DownloadFailed {
                request_id,
                model_id,
                ..
            } => (request_id, Some(model_id)),
        };
        if !valid_request_id(request_id) || model_id.is_some_and(|id| !valid_model_id(id)) {
            return Err(LocalModelFrameError::InvalidIdentifier);
        }
        match self {
            Self::Catalogue { models, .. }
                if models.iter().any(|model| {
                    !valid_model_id(&model.id)
                        || model.label.trim().is_empty()
                        || model.size_bytes == 0
                }) =>
            {
                Err(LocalModelFrameError::InvalidCatalogue)
            }
            Self::StatusResult { state, .. } | Self::DownloadAccepted { state, .. } => {
                valid_state(state)
            }
            Self::DownloadProgress {
                downloaded_bytes,
                total_bytes,
                ..
            } if *total_bytes == 0 || downloaded_bytes > total_bytes => {
                Err(LocalModelFrameError::InvalidProgress)
            }
            Self::DownloadComplete { size_bytes, .. } if *size_bytes == 0 => {
                Err(LocalModelFrameError::InvalidProgress)
            }
            _ => Ok(()),
        }
    }
}

fn valid_state(state: &LocalModelInstallState) -> Result<(), LocalModelFrameError> {
    match state {
        LocalModelInstallState::Downloading {
            downloaded_bytes,
            total_bytes,
        } if *total_bytes == 0 || downloaded_bytes > total_bytes => {
            Err(LocalModelFrameError::InvalidProgress)
        }
        LocalModelInstallState::Ready { size_bytes } if *size_bytes == 0 => {
            Err(LocalModelFrameError::InvalidProgress)
        }
        _ => Ok(()),
    }
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum LocalModelFrameError {
    #[error("invalid local-model request or model identifier")]
    InvalidIdentifier,
    #[error("invalid local-model catalogue descriptor")]
    InvalidCatalogue,
    #[error("invalid local-model download progress")]
    InvalidProgress,
}

#[cfg(test)]
mod tests {
    use super::{LOCAL_MODELS_CAPABILITY, LocalModelFrame, LocalModelInstallState};
    use serde_json::json;

    #[test]
    fn download_progress_is_typed_and_correlated() {
        let frame = LocalModelFrame::DownloadProgress {
            request_id: "download-1".into(),
            model_id: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
            downloaded_bytes: 10,
            total_bytes: 20,
        };
        frame.validate().unwrap();
        assert_eq!(LOCAL_MODELS_CAPABILITY, "local-models-v1");
        assert_eq!(
            serde_json::to_value(frame).unwrap(),
            json!({"type":"downloadProgress","body":{"requestId":"download-1","modelId":"qwen2-5-coder-7b-instruct-q4-k-m","downloadedBytes":10,"totalBytes":20}})
        );
    }

    #[test]
    fn repeated_model_downloads_keep_distinct_request_ids_on_wire() {
        let model_id = "qwen2-5-coder-7b-instruct-q4-k-m";
        let frames = [
            LocalModelFrame::DownloadComplete {
                request_id: "prompt-a".into(),
                model_id: model_id.into(),
                size_bytes: 20,
            },
            LocalModelFrame::DownloadAccepted {
                request_id: "prompt-b".into(),
                model_id: model_id.into(),
                state: LocalModelInstallState::Downloading {
                    downloaded_bytes: 0,
                    total_bytes: 20,
                },
            },
            LocalModelFrame::DownloadProgress {
                request_id: "prompt-b".into(),
                model_id: model_id.into(),
                downloaded_bytes: 10,
                total_bytes: 20,
            },
            LocalModelFrame::DownloadComplete {
                request_id: "prompt-b".into(),
                model_id: model_id.into(),
                size_bytes: 20,
            },
        ];
        let wire_frames = frames
            .iter()
            .map(|frame| serde_json::to_value(frame).unwrap())
            .collect::<Vec<_>>();
        let decoded = wire_frames
            .into_iter()
            .map(|value| serde_json::from_value::<LocalModelFrame>(value).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(decoded, frames);
        assert!(decoded.iter().all(|frame| frame.validate().is_ok()));
    }

    #[test]
    fn requests_cannot_select_urls_paths_or_invalid_model_ids() {
        assert!(
            serde_json::from_value::<LocalModelFrame>(json!({
                "type":"download", "body":{"requestId":"request-1", "modelId":"../../outside"}
            }))
            .unwrap()
            .validate()
            .is_err()
        );
        assert!(serde_json::from_value::<LocalModelFrame>(json!({
            "type":"download", "body":{"requestId":"request-1", "modelId":"model", "url":"https://bad.test"}
        }))
        .is_err());
    }

    #[test]
    fn rejects_contradictory_durable_download_states() {
        assert!(
            LocalModelFrame::StatusResult {
                request_id: "request-1".into(),
                model_id: "model".into(),
                state: LocalModelInstallState::Downloading {
                    downloaded_bytes: 2,
                    total_bytes: 1,
                },
            }
            .validate()
            .is_err()
        );
    }
}
