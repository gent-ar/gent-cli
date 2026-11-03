//! Capability-gated, secret-free provider authentication exchanges.
//!
//! This public endpoint carries only an `askTool` challenge, a method selection, and state.
//! V1 carries no browser handoff. The separately negotiated handoff extension may expose
//! bounded public device-verification instructions; private device secrets, browser callbacks,
//! API keys, access tokens and credential persistence never enter these frames or durable DTOs.

use gent_types::{
    ProviderAuthChallenge, ProviderAuthHandoffMethod, ProviderAuthHandoffState,
    ProviderAuthMethodSelection, ProviderAuthProvider, ProviderAuthStatus,
};
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

/// Required before a client may request public Claude or Codex authentication state.
pub const PROVIDER_AUTH_CAPABILITY: &str = "provider-auth-v1";
pub const PROVIDER_AUTH_HANDOFF_CAPABILITY: &str = "provider-auth-handoff-v1";
/// Maximum encoded size for a provider-auth endpoint frame.
pub const MAX_PROVIDER_AUTH_FRAME_BYTES: usize = 8 * 1024;

/// One bounded, secret-free provider-auth exchange on its dedicated local endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderAuthFrame {
    HandoffStatusRequest {
        request_id: String,
        provider: ProviderAuthProvider,
    },
    HandoffLoginRequest {
        request_id: String,
        provider: ProviderAuthProvider,
    },
    HandoffSelectMethod {
        request_id: String,
        challenge_id: String,
        method: ProviderAuthHandoffMethod,
    },
    HandoffState {
        request_id: String,
        state: ProviderAuthHandoffState,
    },
    StatusRequest {
        request_id: String,
        provider: ProviderAuthProvider,
    },
    /// Explicit user intent to request a typed authentication choice, never credentials.
    LoginRequest {
        request_id: String,
        provider: ProviderAuthProvider,
    },
    Status {
        request_id: String,
        status: ProviderAuthStatus,
    },
    /// Render a typed user choice in either the terminal or a native host UI.
    AskTool {
        request_id: String,
        challenge: ProviderAuthChallenge,
    },
    /// Selects a route; it is never a container for a credential value.
    SelectMethod {
        request_id: String,
        selection: ProviderAuthMethodSelection,
    },
    SelectionAccepted {
        request_id: String,
        status: ProviderAuthStatus,
    },
    Cancel {
        request_id: String,
        challenge_id: String,
    },
}

impl ProviderAuthFrame {
    /// Validates correlation identifiers, typed values, and the exact endpoint byte budget.
    ///
    /// # Errors
    /// Returns an error for malformed values or an oversized encoded frame.
    pub fn validate(&self) -> Result<(), ProviderAuthFrameError> {
        match self {
            Self::HandoffStatusRequest { request_id, .. }
            | Self::HandoffLoginRequest { request_id, .. } => validate_id(request_id)?,
            Self::HandoffSelectMethod {
                request_id,
                challenge_id,
                ..
            } => {
                validate_id(request_id)?;
                validate_id(challenge_id)?;
            }
            Self::HandoffState { request_id, state } => {
                validate_id(request_id)?;
                state.validate()?;
            }
            Self::StatusRequest { request_id, .. } | Self::LoginRequest { request_id, .. } => {
                validate_id(request_id)?;
            }
            Self::Status { request_id, status }
            | Self::SelectionAccepted { request_id, status } => {
                validate_id(request_id)?;
                if let Some(challenge_id) = &status.challenge_id {
                    validate_id(challenge_id)?;
                }
                if let Some(binary_lock) = &status.binary_lock {
                    binary_lock.validate()?;
                }
            }
            Self::AskTool {
                request_id,
                challenge,
            } => {
                validate_id(request_id)?;
                challenge.validate()?;
            }
            Self::SelectMethod {
                request_id,
                selection,
            } => {
                validate_id(request_id)?;
                selection.validate()?;
            }
            Self::Cancel {
                request_id,
                challenge_id,
            } => {
                validate_id(request_id)?;
                validate_id(challenge_id)?;
            }
        }
        if self.encoded_len()? > MAX_PROVIDER_AUTH_FRAME_BYTES {
            return Err(ProviderAuthFrameError::TooLarge);
        }
        Ok(())
    }

    fn encoded_len(&self) -> Result<usize, ProviderAuthFrameError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| ProviderAuthFrameError::InvalidEncoding)
    }
}

/// Writes one validated provider-auth frame within the endpoint's byte budget.
///
/// # Errors
/// Returns invalid data for an invalid or oversized secret-free contract frame.
pub async fn write_provider_auth_frame<W>(
    writer: &mut W,
    frame: &ProviderAuthFrame,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    frame.validate().map_err(io::Error::other)?;
    crate::write_json_frame(writer, frame).await
}

/// Reads one bounded and validated provider-auth frame.
///
/// # Errors
/// Returns invalid data for malformed, oversized, or invalid contract frames.
pub async fn read_provider_auth_frame<R>(reader: &mut R) -> io::Result<ProviderAuthFrame>
where
    R: AsyncRead + Unpin,
{
    let length = usize::try_from(reader.read_u32().await?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"))?;
    if length > MAX_PROVIDER_AUTH_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "provider authentication frame exceeds maximum size",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    let frame = serde_json::from_slice::<ProviderAuthFrame>(&body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    frame.validate().map_err(io::Error::other)?;
    Ok(frame)
}

fn validate_id(value: &str) -> Result<(), ProviderAuthFrameError> {
    if value.is_empty() || value.len() > 128 {
        return Err(ProviderAuthFrameError::InvalidIdentifier);
    }
    Ok(())
}

/// Protocol errors omit all input values, especially credential-like content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProviderAuthFrameError {
    #[error("provider authentication identifier is invalid")]
    InvalidIdentifier,
    #[error("provider authentication value is invalid")]
    InvalidValue,
    #[error("provider authentication frame exceeds byte budget")]
    TooLarge,
    #[error("provider authentication frame could not be encoded")]
    InvalidEncoding,
}

impl From<gent_types::ProviderAuthContractError> for ProviderAuthFrameError {
    fn from(_: gent_types::ProviderAuthContractError) -> Self {
        Self::InvalidValue
    }
}

#[cfg(test)]
#[path = "provider_auth_handoff_tests.rs"]
mod handoff_tests;
#[cfg(test)]
#[path = "provider_auth_tests.rs"]
mod tests;
