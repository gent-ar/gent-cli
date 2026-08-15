//! Public user-message encoding for documented Claude and Codex transports.

use gent_types::Command;
use serde_json::{Value, json};

use crate::discovery::PublicProvider;

/// Provider-native session facts required to encode one user turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicSession {
    /// Claude's stream-JSON input is tied to the launched process.
    ClaudeStream,
    /// Codex app-server requests always name the server-owned thread and request number.
    CodexAppServer { thread_id: String, request_id: u64 },
}

/// Encodes one `userMessage` command as an NDJSON frame for a known public transport.
///
/// # Errors
/// Returns an error for an unsupported command, a missing prompt, or a mismatched session.
pub fn encode_user_message(
    provider: PublicProvider,
    session: &PublicSession,
    command: &Command,
) -> Result<Vec<u8>, MessageEncodingError> {
    let prompt = prompt(command)?;
    let frame = match (provider, session) {
        (PublicProvider::Claude, PublicSession::ClaudeStream) => claude_frame(prompt),
        (
            PublicProvider::Codex,
            PublicSession::CodexAppServer {
                thread_id,
                request_id,
            },
        ) if !thread_id.is_empty() && *request_id > 0 => {
            codex_frame(prompt, thread_id, *request_id)
        }
        (PublicProvider::Codex, PublicSession::CodexAppServer { .. }) => {
            return Err(MessageEncodingError::InvalidCodexSession);
        }
        _ => return Err(MessageEncodingError::SessionProviderMismatch),
    };
    let mut encoded =
        serde_json::to_vec(&frame).map_err(|_| MessageEncodingError::Serialization)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn prompt(command: &Command) -> Result<&str, MessageEncodingError> {
    if command.kind != "userMessage" {
        return Err(MessageEncodingError::UnsupportedCommand);
    }
    command
        .payload
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.is_empty())
        .ok_or(MessageEncodingError::MissingPrompt)
}

fn claude_frame(prompt: &str) -> Value {
    json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": prompt}]}
    })
}

fn codex_frame(prompt: &str, thread_id: &str, request_id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "turn/start",
        "params": {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]}
    })
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum MessageEncodingError {
    #[error("only userMessage commands enter a public provider transport")]
    UnsupportedCommand,
    #[error("a user message requires a non-empty prompt")]
    MissingPrompt,
    #[error("the provider does not match the recorded native session")]
    SessionProviderMismatch,
    #[error("a Codex app-server session requires a thread and positive request identifier")]
    InvalidCodexSession,
    #[error("provider input could not be encoded")]
    Serialization,
}

#[cfg(test)]
mod tests {
    use gent_types::{Command, HostEpoch, ReceiptId};
    use serde_json::{Value, json};

    use super::{MessageEncodingError, PublicProvider, PublicSession, encode_user_message};

    fn command(prompt: &Value) -> Command {
        Command {
            receipt_id: ReceiptId::new(),
            idempotency_key: "key".into(),
            host_epoch: HostEpoch(1),
            kind: "userMessage".into(),
            payload: json!({"prompt": prompt}),
        }
    }

    fn decode(encoded: &[u8]) -> Value {
        assert_eq!(encoded.last(), Some(&b'\n'));
        serde_json::from_slice(&encoded[..encoded.len() - 1]).unwrap()
    }

    #[test]
    fn claude_encodes_a_stream_json_user_frame() {
        assert_eq!(
            decode(
                &encode_user_message(
                    PublicProvider::Claude,
                    &PublicSession::ClaudeStream,
                    &command(&json!("hello")),
                )
                .unwrap(),
            ),
            json!({
                "type": "user",
                "message": {"role": "user", "content": [{"type": "text", "text": "hello"}]}
            })
        );
    }

    #[test]
    fn codex_encodes_a_thread_bound_json_rpc_turn() {
        assert_eq!(
            decode(
                &encode_user_message(
                    PublicProvider::Codex,
                    &PublicSession::CodexAppServer {
                        thread_id: "thread-1".into(),
                        request_id: 7,
                    },
                    &command(&json!("hello")),
                )
                .unwrap(),
            ),
            json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "turn/start",
                "params": {"threadId": "thread-1", "input": [{"type": "text", "text": "hello"}]}
            })
        );
    }

    #[test]
    fn encoding_rejects_invalid_commands_and_sessions_without_echoing_prompt() {
        let mut invalid_kind = command(&json!("secret-like-text"));
        invalid_kind.kind = "ping".into();
        assert_eq!(
            encode_user_message(
                PublicProvider::Claude,
                &PublicSession::ClaudeStream,
                &invalid_kind,
            ),
            Err(MessageEncodingError::UnsupportedCommand)
        );
        assert_eq!(
            encode_user_message(
                PublicProvider::Codex,
                &PublicSession::CodexAppServer {
                    thread_id: String::new(),
                    request_id: 0,
                },
                &command(&json!("hello")),
            ),
            Err(MessageEncodingError::InvalidCodexSession)
        );
        assert_eq!(
            encode_user_message(
                PublicProvider::Claude,
                &PublicSession::CodexAppServer {
                    thread_id: "thread".into(),
                    request_id: 1,
                },
                &command(&json!("hello")),
            ),
            Err(MessageEncodingError::SessionProviderMismatch)
        );
    }
}
