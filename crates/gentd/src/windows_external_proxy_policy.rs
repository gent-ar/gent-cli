use gent_types::AgentChatProvider;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExternalProxyPolicy {
    pub schema: u8,
    pub policy: PolicyKind,
    #[serde(default)]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    pub ingress: IngressScope,
    pub physical_lan_verified: bool,
    pub helper_sha256: String,
    pub resources_sha256: String,
    pub package: PackageIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) enum PolicyKind {
    #[serde(rename = "external-identity-proxy-private-v1")]
    ExternalIdentityProxyPrivateV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) enum IngressScope {
    #[serde(rename = "private-network-capability-accepted")]
    PrivateNetworkCapabilityAccepted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Provider {
    Codex,
    Claude,
    Claurst,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackageIdentity {
    pub family: String,
    pub full_name: String,
    pub application: String,
    pub image_sha256: String,
    pub manifest_sha256: String,
    pub development_registration: bool,
}

impl ExternalProxyPolicy {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 16 * 1024 {
            return Err("reviewed external proxy descriptor exceeds its bound".into());
        }
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|_| "reviewed external proxy descriptor is invalid")?;
        let policy: Self = serde_json::from_slice(bytes)
            .map_err(|_| "reviewed external proxy descriptor is invalid")?;
        let provider_scope_valid = match policy.schema {
            1 => policy.provider.is_some() && value.get("providers").is_none(),
            2 => {
                value.get("provider").is_none()
                    && !policy.providers.is_empty()
                    && policy.providers.len() <= 3
                    && policy
                        .providers
                        .iter()
                        .enumerate()
                        .all(|(index, provider)| !policy.providers[..index].contains(provider))
            }
            _ => false,
        };
        if !provider_scope_valid
            || policy.physical_lan_verified
            || !digest(&policy.helper_sha256)
            || !digest(&policy.resources_sha256)
            || !digest(&policy.package.image_sha256)
            || !digest(&policy.package.manifest_sha256)
            || !identifier(&policy.package.family)
            || !identifier(&policy.package.full_name)
            || !identifier(&policy.package.application)
        {
            return Err(
                "reviewed external proxy descriptor does not match its declared scope".into(),
            );
        }
        Ok(policy)
    }

    pub(crate) fn require(
        &self,
        provider: AgentChatProvider,
        helper_digest: &str,
        resources_digest: Option<&str>,
    ) -> Result<(), String> {
        let selected = match provider {
            AgentChatProvider::Codex => Some(Provider::Codex),
            AgentChatProvider::Claude => Some(Provider::Claude),
            AgentChatProvider::Claurst => Some(Provider::Claurst),
        };
        if !selected.as_ref().is_some_and(|selected| {
            self.provider.as_ref() == Some(selected) || self.providers.contains(selected)
        }) {
            return Err("selected provider has no reviewed external proxy factory".into());
        }
        if self.helper_sha256 != helper_digest
            || resources_digest != Some(self.resources_sha256.as_str())
        {
            return Err("reviewed external proxy artifacts do not match this daemon".into());
        }
        Ok(())
    }

    pub(crate) fn require_package(&self, actual: &PackageIdentity) -> Result<(), String> {
        if self.package != *actual {
            return Err("installed external proxy package differs from reviewed authority".into());
        }
        Ok(())
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
#[path = "windows_external_proxy_policy_tests.rs"]
mod tests;
