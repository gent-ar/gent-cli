use gent_types::ProviderAuthHandoff;
use serde_json::{Value, json};

pub(super) struct CodexAuthState {
    pub(super) handoff: Option<ProviderAuthHandoff>,
    pub(super) completed: Option<bool>,
    login_id: Option<String>,
    early_completions: Vec<(String, bool)>,
    initialized: bool,
}

impl CodexAuthState {
    pub(super) fn new() -> Self {
        Self {
            handoff: None,
            completed: None,
            login_id: None,
            early_completions: Vec::new(),
            initialized: false,
        }
    }

    pub(super) fn initialize() -> Value {
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "clientInfo":{"name":"gent-provider-auth","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true,"requestAttestation":false}}})
    }

    pub(super) fn receive(&mut self, value: &Value, expiry: u64) -> Result<Vec<Value>, ()> {
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            if method == "account/login/completed" {
                if self.completed.is_some() {
                    return Ok(Vec::new());
                }
                let Some(params) = value.get("params") else {
                    return Ok(Vec::new());
                };
                let Some(login_id) = params.get("loginId").and_then(|id| bounded_id(id).ok())
                else {
                    return Ok(Vec::new());
                };
                let Some(success) = params.get("success").and_then(Value::as_bool) else {
                    return Ok(Vec::new());
                };
                if self.login_id.as_deref() == Some(login_id) {
                    self.completed = Some(success);
                    self.handoff = None;
                } else if self.login_id.is_none() {
                    if self.early_completions.len() < 8
                        && !self.early_completions.iter().any(|(id, _)| id == login_id)
                    {
                        self.early_completions.push((login_id.into(), success));
                    }
                }
            }
            // Auth-refresh/server requests are never answered with credentials.
            return Ok(value.get("id").filter(|id| id.is_string() || id.is_u64()).map(|id|
                vec![json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported auth-only client request"}})]
            ).unwrap_or_default());
        }
        match value.get("id").and_then(Value::as_u64) {
            Some(1) if !self.initialized => {
                if value.get("error").is_some()
                    || !value.get("result").is_some_and(Value::is_object)
                {
                    return Err(());
                }
                self.initialized = true;
                Ok(vec![
                    json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
                    json!({"jsonrpc":"2.0","id":2,"method":"account/login/start","params":{"type":"chatgptDeviceCode"}}),
                ])
            }
            Some(2) if self.initialized && self.login_id.is_none() => {
                if value.get("error").is_some() {
                    return Err(());
                }
                let result = value.get("result").ok_or(())?;
                if result.get("type").and_then(Value::as_str) != Some("chatgptDeviceCode") {
                    return Err(());
                }
                let login_id = bounded_id(result.get("loginId").ok_or(())?)?.to_owned();
                let url = result
                    .get("verificationUrl")
                    .and_then(Value::as_str)
                    .ok_or(())?;
                if !url.starts_with("https://auth.openai.com/") {
                    return Err(());
                }
                let handoff = ProviderAuthHandoff::DeviceVerification {
                    verification_url: url.into(),
                    user_code: result
                        .get("userCode")
                        .and_then(Value::as_str)
                        .ok_or(())?
                        .into(),
                    expires_at_unix_seconds: expiry,
                };
                handoff.validate().map_err(|_| ())?;
                self.completed = self
                    .early_completions
                    .iter()
                    .find(|(id, _)| id == &login_id)
                    .map(|(_, success)| *success);
                self.early_completions.clear();
                self.login_id = Some(login_id);
                if self.completed.is_none() {
                    self.handoff = Some(handoff);
                }
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(super) fn cancel(&self) -> Option<Value> {
        self.login_id.as_ref().map(|id| json!({"jsonrpc":"2.0","id":3,"method":"account/login/cancel","params":{"loginId":id}}))
    }
}

fn bounded_id(value: &Value) -> Result<&str, ()> {
    value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .ok_or(())
}

#[cfg(test)]
#[path = "provider_auth_codex_state_tests.rs"]
mod tests;
