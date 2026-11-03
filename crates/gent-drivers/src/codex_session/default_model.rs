//! Resolve the durable provider-default alias from this newly connected SDK.
use super::phase::CodexSessionPhase;
use super::wire::{encode, thread_request};
use super::{CodexAppServerSession, CodexSessionConfig, CodexSessionError, CodexSessionIngress};
use serde_json::{Value, json};

const MAX_PAGES: u8 = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DefaultModelDiscovery {
    config: CodexSessionConfig,
    page: u8,
    candidate: Option<String>,
    cursors: Vec<String>,
}

impl DefaultModelDiscovery {
    pub(super) fn new(config: CodexSessionConfig) -> Self {
        Self {
            config,
            page: 1,
            candidate: None,
            cursors: Vec::new(),
        }
    }
}

pub(super) fn request(id: u64, cursor: Option<&str>) -> Result<Vec<u8>, CodexSessionError> {
    encode(
        &json!({"id": id, "method": "model/list", "params": {"cursor": cursor, "includeHidden": false}}),
    )
}

impl CodexAppServerSession {
    pub(super) fn settle_provider_default(
        &mut self,
        mut discovery: DefaultModelDiscovery,
        frame: &Value,
    ) -> Result<CodexSessionIngress, CodexSessionError> {
        // Parsing or bounds failure must never release a thread/user prompt.
        self.phase = CodexSessionPhase::Failed;
        let result = frame
            .get("result")
            .ok_or(CodexSessionError::MalformedResponse)?;
        let models = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or(CodexSessionError::MalformedResponse)?;
        for model in models {
            if model.get("hidden").and_then(Value::as_bool) == Some(true)
                || model.get("isDefault").and_then(Value::as_bool) != Some(true)
            {
                continue;
            }
            let name = model
                .get("model")
                .or_else(|| model.get("id"))
                .and_then(Value::as_str)
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= 512
                        && !name.contains('\0')
                        && *name != "default"
                })
                .ok_or(CodexSessionError::ProviderDefaultUnavailable)?;
            if discovery
                .candidate
                .as_deref()
                .is_some_and(|previous| previous != name)
            {
                return Err(CodexSessionError::ProviderDefaultUnavailable);
            }
            discovery.candidate = Some(name.to_owned());
        }
        let cursor = match result.get("nextCursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if cursor.is_empty() => None,
            Some(Value::String(cursor)) if cursor.len() <= 512 && !cursor.contains('\0') => {
                Some(cursor.clone())
            }
            _ => return Err(CodexSessionError::MalformedResponse),
        };
        if let Some(cursor) = cursor {
            if discovery.page >= MAX_PAGES || discovery.cursors.contains(&cursor) {
                return Err(CodexSessionError::ProviderDefaultUnavailable);
            }
            discovery.page += 1;
            discovery.cursors.push(cursor.clone());
            let request_id = self.take_request_id()?;
            self.phase = CodexSessionPhase::AwaitDefaultModel {
                request_id,
                discovery,
            };
            return Ok(CodexSessionIngress::Send(vec![request(
                request_id,
                Some(&cursor),
            )?]));
        }
        let model = discovery
            .candidate
            .ok_or(CodexSessionError::ProviderDefaultUnavailable)?;
        discovery.config.turn_options = discovery
            .config
            .turn_options
            .resolve_provider_default(model.clone())?;
        let (method, mut params, resumed_thread_id, turn_options) =
            thread_request(discovery.config);
        params["model"] = Value::String(model);
        let request_id = self.take_request_id()?;
        self.phase = CodexSessionPhase::AwaitThread {
            request_id,
            resumed_thread_id,
            turn_options,
        };
        Ok(CodexSessionIngress::Send(vec![encode(
            &json!({"id": request_id, "method": method, "params": params}),
        )?]))
    }
}

#[cfg(test)]
#[path = "default_model_tests.rs"]
mod tests;
