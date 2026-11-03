use std::{fmt, path::PathBuf};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

use super::EgressError;

pub(super) struct CodexOwnerContext {
    pub(super) home: PathBuf,
    account_kind: String,
    email: Option<String>,
    workspace_ids: Vec<String>,
}

pub(super) struct CodexOwnerCredential {
    pub(super) upstream: &'static str,
    pub(super) headers: Vec<(String, String)>,
}

impl fmt::Debug for CodexOwnerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CodexOwnerCredential { credentials: <redacted> }")
    }
}

impl CodexOwnerContext {
    pub(super) fn configuration_home(config: &Value) -> Result<PathBuf, EgressError> {
        let effective = config
            .get("config")
            .and_then(Value::as_object)
            .ok_or(EgressError::Authentication)?;
        if effective
            .get("cli_auth_credentials_store")
            .is_some_and(|value| value.as_str() != Some("file"))
            || effective
                .get("model_provider")
                .is_some_and(|value| !value.is_null() && value.as_str() != Some("openai"))
            || effective.get("chatgpt_base_url").is_some_and(|value| {
                !value.is_null()
                    && !matches!(
                        value.as_str(),
                        Some(
                            "https://chatgpt.com/backend-api" | "https://chatgpt.com/backend-api/"
                        )
                    )
            })
        {
            return Err(EgressError::Authentication);
        }
        let layers = config
            .get("layers")
            .and_then(Value::as_array)
            .ok_or(EgressError::Authentication)?;
        if layers.len() > 128 {
            return Err(EgressError::Authentication);
        }
        let mut homes = std::collections::BTreeSet::new();
        for layer in layers {
            if layer.pointer("/name/type").and_then(Value::as_str) != Some("user") {
                continue;
            }
            if layer
                .get("disabledReason")
                .is_some_and(|value| !value.is_null())
            {
                return Err(EgressError::Authentication);
            }
            let file = layer
                .pointer("/name/file")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or(EgressError::Authentication)?;
            if !file.is_absolute()
                || file.file_name().and_then(|name| name.to_str()) != Some("config.toml")
            {
                return Err(EgressError::Authentication);
            }
            homes.insert(
                file.parent()
                    .ok_or(EgressError::Authentication)?
                    .to_path_buf(),
            );
        }
        if homes.len() != 1 {
            return Err(EgressError::Authentication);
        }
        homes.into_iter().next().ok_or(EgressError::Authentication)
    }

    pub(super) fn parse(config: &Value, account: &Value) -> Result<Self, EgressError> {
        let home = Self::configuration_home(config)?;
        let effective = config
            .get("config")
            .and_then(Value::as_object)
            .ok_or(EgressError::Authentication)?;
        let account = account
            .get("account")
            .filter(|value| value.is_object())
            .ok_or(EgressError::Authentication)?;
        let account_kind = account
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| matches!(*kind, "apiKey" | "chatgpt"))
            .ok_or(EgressError::Authentication)?
            .to_owned();
        let email = account
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let workspace_ids = match effective.get("forced_chatgpt_workspace_id") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(value)) => vec![value.clone()],
            Some(Value::Array(values)) if values.len() <= 32 => values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(EgressError::Authentication)
                })
                .collect::<Result<_, _>>()?,
            _ => return Err(EgressError::Authentication),
        };
        Ok(Self {
            home,
            account_kind,
            email,
            workspace_ids,
        })
    }

    pub(super) fn credential(
        &self,
        bytes: &[u8],
        now: u64,
    ) -> Result<CodexOwnerCredential, EgressError> {
        if bytes.len() > 65_536 {
            return Err(EgressError::Authentication);
        }
        let auth: Value = serde_json::from_slice(bytes).map_err(|_| EgressError::Authentication)?;
        if ["agent_identity", "personal_access_token", "bedrock_api_key"]
            .iter()
            .any(|name| auth.get(*name).is_some_and(|value| !value.is_null()))
        {
            return Err(EgressError::Authentication);
        }
        match self.account_kind.as_str() {
            "apiKey" => {
                if auth
                    .get("auth_mode")
                    .is_some_and(|value| !value.is_null() && value.as_str() != Some("apikey"))
                {
                    return Err(EgressError::Authentication);
                }
                let key = secret(auth.get("OPENAI_API_KEY").and_then(Value::as_str))?;
                Ok(CodexOwnerCredential {
                    upstream: "https://api.openai.com/v1/",
                    headers: vec![("authorization".into(), format!("Bearer {key}"))],
                })
            }
            "chatgpt" => self.chatgpt(&auth, now),
            _ => Err(EgressError::Authentication),
        }
    }

    fn chatgpt(&self, auth: &Value, now: u64) -> Result<CodexOwnerCredential, EgressError> {
        if auth
            .get("auth_mode")
            .is_some_and(|value| !value.is_null() && value.as_str() != Some("chatgpt"))
            || auth
                .get("OPENAI_API_KEY")
                .is_some_and(|value| !value.is_null())
        {
            return Err(EgressError::Authentication);
        }
        let token = secret(auth.pointer("/tokens/access_token").and_then(Value::as_str))?;
        let access_claims = claims(token)?;
        let expiry = access_claims
            .get("exp")
            .and_then(Value::as_u64)
            .ok_or(EgressError::Authentication)?;
        if expiry <= now.saturating_add(60) {
            return Err(EgressError::Authentication);
        }
        let identity = claims(secret(
            auth.pointer("/tokens/id_token").and_then(Value::as_str),
        )?)?;
        if [&identity, &access_claims].into_iter().any(|claims| {
            claims
                .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_is_fedramp")
                .and_then(Value::as_bool)
                == Some(true)
        }) {
            return Err(EgressError::Authentication);
        }
        let email = identity.get("email").and_then(Value::as_str).or_else(|| {
            identity
                .pointer("/https:~1~1api.openai.com~1profile/email")
                .and_then(Value::as_str)
        });
        if self
            .email
            .as_deref()
            .is_none_or(|expected| email != Some(expected))
        {
            return Err(EgressError::Authentication);
        }
        let account = auth
            .pointer("/tokens/account_id")
            .and_then(Value::as_str)
            .or_else(|| {
                identity
                    .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
                    .and_then(Value::as_str)
            })
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 256
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
            .ok_or(EgressError::Authentication)?;
        if !self.workspace_ids.is_empty()
            && !self.workspace_ids.iter().any(|value| value == account)
        {
            return Err(EgressError::Authentication);
        }
        Ok(CodexOwnerCredential {
            upstream: "https://chatgpt.com/backend-api/codex/",
            headers: vec![
                ("authorization".into(), format!("Bearer {token}")),
                ("chatgpt-account-id".into(), account.into()),
            ],
        })
    }
}

fn secret(value: Option<&str>) -> Result<&str, EgressError> {
    value
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 16_384
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .ok_or(EgressError::Authentication)
}

fn claims(token: &str) -> Result<Value, EgressError> {
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(EgressError::Authentication);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| EgressError::Authentication)?;
    serde_json::from_slice(&payload).map_err(|_| EgressError::Authentication)
}

#[cfg(test)]
#[path = "provider_egress_codex_owner_tests.rs"]
mod tests;
