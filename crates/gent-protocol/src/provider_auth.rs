//! Capability-gated, secret-free provider authentication exchanges.
//!
//! This public endpoint carries only an `askTool` challenge, a method selection, and state.
//! Browser callbacks, device codes, API keys, access tokens, and credential persistence stay
//! outside Gent IPC and durable DTOs at a future provider-owned execution edge.

use gent_types::{
    ProviderAuthChallenge, ProviderAuthMethodSelection, ProviderAuthProvider, ProviderAuthStatus,
};
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

/// Required before a client may request public Claude or Codex authentication state.
pub const PROVIDER_AUTH_CAPABILITY: &str = "provider-auth-v1";
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
            Self::StatusRequest { request_id, .. } | Self::LoginRequest { request_id, .. } => {
                validate_id(request_id)?;
            }
            Self::Status { request_id, status }
            | Self::SelectionAccepted { request_id, status } => {
                validate_id(request_id)?;
                status.binary_lock.validate()?;
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
mod tests {
    use super::{
        MAX_PROVIDER_AUTH_FRAME_BYTES, PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame,
        ProviderAuthFrameError,
    };
    use gent_types::{
        ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthLifecycle, ProviderAuthMethod,
        ProviderAuthProvider, ProviderAuthStatus,
    };
    use serde_json::{Value, json};

    fn lock() -> ProviderAuthBinaryLock {
        ProviderAuthBinaryLock {
            canonical_executable_id: "provider-id:claude".into(),
            digest_sha256: "a".repeat(64),
            version: "1.2.3".into(),
        }
    }

    #[test]
    fn ask_tool_is_typed_and_bounded() {
        let frame = ProviderAuthFrame::AskTool {
            request_id: "request-1".into(),
            challenge: ProviderAuthChallenge {
                challenge_id: "challenge-1".into(),
                provider: ProviderAuthProvider::Claude,
                binary_lock: lock(),
                methods: vec![
                    ProviderAuthMethod::AccountBrowser,
                    ProviderAuthMethod::ApiKey,
                ],
                expires_at_unix_seconds: 42,
            },
        };
        let encoded = serde_json::to_vec(&frame).unwrap();
        assert!(encoded.len() <= MAX_PROVIDER_AUTH_FRAME_BYTES);
        assert_eq!(frame.validate(), Ok(()));
        assert_eq!(PROVIDER_AUTH_CAPABILITY, "provider-auth-v1");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let frame = json!({
            "type": "selectMethod", "body": {
                "requestId": "request-1", "selection": {
                    "challengeId": "challenge-1", "method": "accountBrowser", "apiKey": "never"
                }
            }
        });
        assert!(serde_json::from_value::<ProviderAuthFrame>(frame).is_err());
    }

    #[test]
    fn client_requests_cannot_supply_a_binary_lock_and_login_is_explicit() {
        let frame = json!({
            "type": "statusRequest", "body": {
                "requestId": "request-1", "provider": "claude", "binaryLock": {}
            }
        });
        assert!(serde_json::from_value::<ProviderAuthFrame>(frame).is_err());
        assert_eq!(
            ProviderAuthFrame::LoginRequest {
                request_id: "request-1".into(),
                provider: ProviderAuthProvider::Claude,
            }
            .validate(),
            Ok(())
        );
    }

    #[test]
    fn serialized_status_has_no_secret_bearing_property_names() {
        let frame = ProviderAuthFrame::Status {
            request_id: "request-1".into(),
            status: ProviderAuthStatus {
                provider: ProviderAuthProvider::Codex,
                binary_lock: lock(),
                lifecycle: ProviderAuthLifecycle::Authenticated,
                selected_method: Some(ProviderAuthMethod::AccessToken),
                expires_at_unix_seconds: Some(42),
            },
        };
        assert_no_secret_keys(&serde_json::to_value(frame).unwrap());
    }

    #[test]
    fn an_oversized_request_is_rejected_before_transport() {
        let frame = ProviderAuthFrame::Cancel {
            request_id: "r".repeat(129),
            challenge_id: "challenge-1".into(),
        };
        assert_eq!(
            frame.validate(),
            Err(ProviderAuthFrameError::InvalidIdentifier)
        );
    }

    fn assert_no_secret_keys(value: &Value) {
        const FORBIDDEN: [&str; 6] = [
            "apiKey",
            "accessToken",
            "token",
            "secret",
            "password",
            "credential",
        ];
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    assert!(
                        !FORBIDDEN.contains(&key.as_str()),
                        "forbidden property: {key}"
                    );
                    assert_no_secret_keys(nested);
                }
            }
            Value::Array(items) => items.iter().for_each(assert_no_secret_keys),
            _ => {}
        }
    }
}
