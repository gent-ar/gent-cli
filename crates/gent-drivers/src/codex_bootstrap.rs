//! Pure Codex app-server thread bootstrap encoding.
//!
//! The process owner waits for the `initialize` response, then writes exactly one of these
//! requests. It owns request ordering, process I/O, and durable session binding; this module
//! only prevents a caller from constructing an invalid public JSON-RPC frame.

use serde_json::{Value, json};

/// The thread operation required after the Codex app-server initialization handshake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexThreadRequest {
    /// Creates a new provider-native thread.
    Start { working_directory: Option<String> },
    /// Reattaches to the exact provider-native thread already recorded by the daemon.
    Resume {
        thread_id: String,
        working_directory: Option<String>,
    },
}

/// Encodes one documented Codex app-server thread request as a newline-delimited JSON-RPC frame.
///
/// # Errors
/// Returns an error for a zero request ID, an empty resume identity, or an explicitly empty
/// working-directory value. No credential, model, policy, or provider-session state is inferred.
pub fn encode_codex_thread_request(
    request_id: u64,
    request: &CodexThreadRequest,
) -> Result<Vec<u8>, CodexBootstrapError> {
    if request_id == 0 {
        return Err(CodexBootstrapError::InvalidRequestId);
    }
    let (method, params) = match request {
        CodexThreadRequest::Start { working_directory } => (
            "thread/start",
            directory_params(working_directory.as_ref())?,
        ),
        CodexThreadRequest::Resume {
            thread_id,
            working_directory,
        } => {
            if thread_id.is_empty() {
                return Err(CodexBootstrapError::EmptyThreadId);
            }
            let mut params = directory_params(working_directory.as_ref())?;
            params["threadId"] = Value::String(thread_id.clone());
            ("thread/resume", params)
        }
    };
    encode_frame(&json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    }))
}

fn directory_params(working_directory: Option<&String>) -> Result<Value, CodexBootstrapError> {
    match working_directory {
        Some(directory) if directory.is_empty() => Err(CodexBootstrapError::EmptyWorkingDirectory),
        Some(directory) => Ok(json!({"cwd": directory})),
        None => Ok(json!({})),
    }
}

fn encode_frame(frame: &Value) -> Result<Vec<u8>, CodexBootstrapError> {
    let mut encoded = serde_json::to_vec(frame).map_err(|_| CodexBootstrapError::Serialization)?;
    encoded.push(b'\n');
    Ok(encoded)
}

/// Controlled failures while encoding the fixed public bootstrap protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexBootstrapError {
    #[error("a Codex app-server request requires a positive JSON-RPC request identifier")]
    InvalidRequestId,
    #[error("a Codex thread resume requires a non-empty provider-native thread identity")]
    EmptyThreadId,
    #[error("an explicitly selected Codex working directory must be non-empty")]
    EmptyWorkingDirectory,
    #[error("the Codex bootstrap frame could not be encoded")]
    Serialization,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{CodexBootstrapError, CodexThreadRequest, encode_codex_thread_request};

    fn decode(encoded: &[u8]) -> Value {
        assert_eq!(encoded.last(), Some(&b'\n'));
        serde_json::from_slice(&encoded[..encoded.len() - 1]).unwrap()
    }

    #[test]
    fn start_encodes_only_the_optional_public_working_directory() {
        assert_eq!(
            decode(
                &encode_codex_thread_request(
                    2,
                    &CodexThreadRequest::Start {
                        working_directory: Some("/work".into()),
                    },
                )
                .unwrap(),
            ),
            json!({"jsonrpc":"2.0","id":2,"method":"thread/start","params":{"cwd":"/work"}})
        );
    }

    #[test]
    fn resume_keeps_the_durable_thread_identity_explicit() {
        assert_eq!(
            decode(
                &encode_codex_thread_request(
                    3,
                    &CodexThreadRequest::Resume {
                        thread_id: "thread-1".into(),
                        working_directory: None,
                    },
                )
                .unwrap(),
            ),
            json!({"jsonrpc":"2.0","id":3,"method":"thread/resume","params":{"threadId":"thread-1"}})
        );
    }

    #[test]
    fn bootstrap_rejects_missing_identities_without_echoing_them() {
        assert_eq!(
            encode_codex_thread_request(
                0,
                &CodexThreadRequest::Start {
                    working_directory: None
                }
            ),
            Err(CodexBootstrapError::InvalidRequestId)
        );
        assert_eq!(
            encode_codex_thread_request(
                1,
                &CodexThreadRequest::Resume {
                    thread_id: String::new(),
                    working_directory: None,
                },
            ),
            Err(CodexBootstrapError::EmptyThreadId)
        );
        assert_eq!(
            encode_codex_thread_request(
                1,
                &CodexThreadRequest::Start {
                    working_directory: Some(String::new()),
                },
            ),
            Err(CodexBootstrapError::EmptyWorkingDirectory)
        );
    }
}
