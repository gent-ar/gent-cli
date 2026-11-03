use std::path::{Component, Path, PathBuf, Prefix};

use serde_json::{Value, json};

use super::{EgressError, codex_owner::CodexOwnerContext};

pub(super) enum Progress {
    Continue,
    RequestAccount(Vec<u8>),
    Done(CodexOwnerContext),
}

pub(super) struct CodexCredentialProbe {
    workspace: PathBuf,
    config: Option<Value>,
}

impl CodexCredentialProbe {
    pub(super) fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.into(),
            config: None,
        }
    }

    pub(super) fn configuration_request() -> Result<Vec<u8>, EgressError> {
        frame(2, "config/read", json!({"includeLayers":true}))
    }

    pub(super) fn accept(&mut self, value: &Value) -> Result<Progress, EgressError> {
        let Some(id @ (2 | 3)) = value.get("id").and_then(Value::as_u64) else {
            return Ok(Progress::Continue);
        };
        if value.get("error").is_some() {
            return Err(EgressError::Authentication);
        }
        let result = value
            .get("result")
            .filter(|value| value.is_object())
            .ok_or(EgressError::Authentication)?;
        if id == 2 {
            if self.config.is_some() {
                return Err(EgressError::Authentication);
            }
            let home = CodexOwnerContext::configuration_home(result)?;
            validate_home(&home, &self.workspace)?;
            self.config = Some(result.clone());
            return frame(3, "account/read", json!({"refreshToken":true}))
                .map(Progress::RequestAccount);
        }
        CodexOwnerContext::parse(
            self.config.as_ref().ok_or(EgressError::Authentication)?,
            result,
        )
        .map(Progress::Done)
    }
}

pub(super) fn validate_home(native: &Path, workspace: &Path) -> Result<PathBuf, EgressError> {
    let canonical = native
        .canonicalize()
        .map_err(|_| EgressError::Authentication)?;
    if !matches!(canonical.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(EgressError::Authentication);
    }
    let normalized = |path: &Path| {
        let value = path.to_string_lossy().replace('/', "\\");
        value
            .strip_prefix("\\\\?\\")
            .unwrap_or(&value)
            .to_ascii_lowercase()
    };
    if normalized(&canonical) != normalized(native)
        || canonical.starts_with(workspace)
        || workspace.starts_with(&canonical)
    {
        return Err(EgressError::Authentication);
    }
    Ok(canonical)
}

fn frame(id: u64, method: &str, params: Value) -> Result<Vec<u8>, EgressError> {
    let mut frame =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .map_err(|_| EgressError::Authentication)?;
    frame.push(b'\n');
    Ok(frame)
}

#[cfg(test)]
#[path = "provider_egress_codex_probe_tests.rs"]
mod tests;
