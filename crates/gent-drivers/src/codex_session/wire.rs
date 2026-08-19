//! Bounded Codex request and identity encoding helpers.

use serde_json::{Value, json};

use super::{
    CodexSessionConfig, CodexSessionError, CodexTurnOptions, MAX_NATIVE_ID_BYTES,
    MAX_WORKING_DIRECTORY_BYTES,
};

pub(super) fn validate_config(config: &CodexSessionConfig) -> Result<(), CodexSessionError> {
    optional_bounded(
        config.working_directory.as_deref(),
        MAX_WORKING_DIRECTORY_BYTES,
    )
    .then_some(())
    .ok_or(CodexSessionError::InvalidWorkingDirectory)?;
    optional_bounded(config.resume_thread_id.as_deref(), MAX_NATIVE_ID_BYTES)
        .then_some(())
        .ok_or(CodexSessionError::InvalidThreadId)
}

pub(super) fn thread_request(
    config: CodexSessionConfig,
) -> (&'static str, Value, Option<String>, CodexTurnOptions) {
    let mut params = config
        .working_directory
        .map_or_else(|| json!({}), |cwd| json!({"cwd": cwd}));
    let turn_options = config.turn_options;
    match config.resume_thread_id {
        Some(thread_id) => {
            params["threadId"] = Value::String(thread_id.clone());
            ("thread/resume", params, Some(thread_id), turn_options)
        }
        None => ("thread/start", params, None, turn_options),
    }
}

pub(super) fn response_id_at(frame: &Value, key: &str) -> Result<String, CodexSessionError> {
    frame
        .get("result")
        .and_then(|result| nested_id(result, key).ok())
        .ok_or(CodexSessionError::MalformedResponse)
}

pub(super) fn nested_id(value: &Value, key: &str) -> Result<String, CodexSessionError> {
    value
        .get(key)
        .and_then(Value::as_object)
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= MAX_NATIVE_ID_BYTES)
        .map(str::to_owned)
        .ok_or(CodexSessionError::MalformedResponse)
}

pub(super) fn encode(frame: &Value) -> Result<Vec<u8>, CodexSessionError> {
    let mut frame = frame.clone();
    frame["jsonrpc"] = Value::String("2.0".into());
    let mut encoded = serde_json::to_vec(&frame).map_err(|_| CodexSessionError::Serialization)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn optional_bounded(value: Option<&str>, maximum: usize) -> bool {
    value.is_none_or(|value| !value.is_empty() && value.len() <= maximum)
}
