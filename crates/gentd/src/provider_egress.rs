use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

#[path = "provider_egress_bound_process.rs"]
mod bound_process;
#[path = "provider_egress_claude_credential.rs"]
mod claude_credential;
#[cfg(windows)]
#[path = "provider_egress_codex_credential.rs"]
mod codex_credential;
#[cfg(windows)]
#[path = "provider_egress_codex_owner.rs"]
mod codex_owner;
#[cfg(windows)]
#[path = "provider_egress_codex_probe.rs"]
mod codex_probe;
#[path = "provider_egress_composition.rs"]
mod composition;
#[path = "provider_egress_credential_capture.rs"]
mod credential_capture;
#[path = "provider_mcp_bridge.rs"]
pub(crate) mod mcp;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_authorizer.rs"]
mod mcp_authorizer;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_composition.rs"]
mod mcp_composition;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_deferred.rs"]
mod mcp_deferred;
#[path = "provider_mcp_http.rs"]
pub(crate) mod mcp_http;
#[path = "provider_mcp_journal.rs"]
pub(crate) mod mcp_journal;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_operations.rs"]
mod mcp_operations;
#[cfg(all(test, windows))]
#[path = "provider_mcp_operations_tests.rs"]
mod mcp_operations_tests;
#[cfg(windows)]
#[path = "provider_mcp_owner.rs"]
mod mcp_owner;
#[cfg(windows)]
#[path = "provider_mcp_paths.rs"]
pub(crate) mod mcp_paths;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_permissions.rs"]
mod mcp_permissions;
#[cfg(windows)]
#[path = "provider_mcp_profiles.rs"]
mod mcp_profiles;
#[cfg(test)]
#[path = "provider_mcp_scope_tests.rs"]
mod mcp_scope_tests;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_secrets.rs"]
mod mcp_secrets;
#[cfg(any(windows, target_os = "linux"))]
#[path = "provider_mcp_stdio.rs"]
mod mcp_stdio;
#[cfg(all(test, windows))]
#[path = "provider_mcp_stdio_scope_tests.rs"]
mod mcp_stdio_scope_tests;
#[path = "provider_mcp_subset.rs"]
mod mcp_subset;
#[path = "provider_egress_probe_process.rs"]
mod probe_process;
#[path = "provider_egress_run_fence.rs"]
pub(crate) mod run_fence;
#[path = "provider_egress_server.rs"]
mod server;
#[path = "provider_egress_wire.rs"]
mod wire;
pub use composition::ProviderGatewayLease;
pub use server::ProviderEgressBroker;
#[cfg(test)]
#[path = "provider_egress_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "provider_egress_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderApi {
    Claude,
    #[cfg(windows)]
    Codex,
    Claurst,
}

enum ProviderTarget {
    Http {
        upstream: Url,
        client: Client,
    },
    #[cfg(windows)]
    ContainedModel(crate::contained_model::ModelEndpoint),
}

pub struct ReviewedProviderEgress {
    api: ProviderApi,
    target: ProviderTarget,
    credentials: HeaderMap,
    policy_digest: String,
}

impl std::fmt::Debug for ReviewedProviderEgress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReviewedProviderEgress")
            .field("api", &self.api)
            .field("policy_digest", &self.policy_digest)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialProbePhase {
    Preparation,
    Busy,
    ScopeUnknown,
    StatusExecution,
    StatusProtocol,
    LoginUnavailable,
    CaptureTransport,
    CaptureProtocol,
    Transcript,
    Cleanup,
}
impl std::fmt::Display for CredentialProbePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Preparation => "owned workspace preparation failed",
            Self::Busy => "another credential check is in progress",
            Self::ScopeUnknown => "process settlement is unproven; restart Gent before retrying",
            Self::StatusExecution => "SDK status check could not complete",
            Self::StatusProtocol => "SDK status response is invalid",
            Self::LoginUnavailable => "no supported Claude.ai login is active",
            Self::CaptureTransport => "local credential transport could not complete",
            Self::CaptureProtocol => "SDK credential response is invalid",
            Self::Transcript => "SDK created an unexpected transcript",
            Self::Cleanup => "owned probe artifacts could not be removed",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    #[error("Claude credential probe failed: {phase}")]
    CredentialProbe {
        phase: CredentialProbePhase,
        cleanup_failed: bool,
        cleanup_unproven: bool,
    },
    #[error("provider egress configuration rejected")]
    Configuration,
    #[error("provider egress request rejected")]
    Request,
    #[error("provider egress authentication rejected")]
    Authentication,
    #[error("provider egress transport failed")]
    Transport,
}

impl ReviewedProviderEgress {
    pub fn new(
        api: ProviderApi,
        upstream: &str,
        credentials: Vec<(String, String)>,
    ) -> Result<Self, EgressError> {
        if api == ProviderApi::Claurst {
            return Err(EgressError::Configuration);
        }
        let upstream = Url::parse(upstream).map_err(|_| EgressError::Configuration)?;
        let host = upstream.host_str().ok_or(EgressError::Configuration)?;
        if upstream.scheme() != "https"
            || !upstream.username().is_empty()
            || upstream.password().is_some()
            || upstream.query().is_some()
            || upstream.fragment().is_some()
            || !upstream.path().ends_with('/')
        {
            return Err(EgressError::Configuration);
        }
        let port = upstream
            .port_or_known_default()
            .ok_or(EgressError::Configuration)?;
        let addresses: Vec<_> = (host, port)
            .to_socket_addrs()
            .map_err(|_| EgressError::Configuration)?
            .filter(|address| address.is_ipv4())
            .collect();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|address| !public_address(address.ip()))
        {
            return Err(EgressError::Configuration);
        }
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300))
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| EgressError::Configuration)?;
        Self::from_client(api, upstream, credentials, client, &addresses)
    }

    fn from_client(
        api: ProviderApi,
        upstream: Url,
        credentials: Vec<(String, String)>,
        client: Client,
        addresses: &[SocketAddr],
    ) -> Result<Self, EgressError> {
        let mut headers = HeaderMap::new();
        for (name, value) in credentials {
            if !matches!(
                name.as_str(),
                "authorization"
                    | "x-api-key"
                    | "anthropic-beta"
                    | "anthropic-version"
                    | "chatgpt-account-id"
                    | "openai-beta"
                    | "originator"
                    | "user-agent"
            ) {
                return Err(EgressError::Configuration);
            }
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| EgressError::Configuration)?;
            let mut value =
                HeaderValue::from_str(&value).map_err(|_| EgressError::Configuration)?;
            value.set_sensitive(true);
            if headers.insert(name, value).is_some() {
                return Err(EgressError::Configuration);
            }
        }
        if !headers.contains_key("authorization") && !headers.contains_key("x-api-key") {
            return Err(EgressError::Configuration);
        }
        let mut hash = Sha256::new();
        hash.update(format!("gateway-v1\0{api:?}\0{upstream}\0"));
        let mut addresses = addresses.to_vec();
        addresses.sort();
        addresses.dedup();
        for address in addresses {
            hash.update(format!("{address}\0"));
        }
        for name in [
            "anthropic-beta",
            "anthropic-version",
            "chatgpt-account-id",
            "openai-beta",
            "originator",
            "user-agent",
        ] {
            if let Some(value) = headers.get(name) {
                hash.update(name.as_bytes());
                hash.update([0]);
                hash.update(value.as_bytes());
                hash.update([0]);
            }
        }
        Ok(Self {
            api,
            target: ProviderTarget::Http { upstream, client },
            credentials: headers,
            policy_digest: format!("{:x}", hash.finalize()),
        })
    }

    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
}

#[path = "provider_egress_addresses.rs"]
mod addresses;
use addresses::public_address;

#[path = "provider_egress_forward.rs"]
mod forward;

#[cfg(target_os = "linux")]
pub(crate) use composition::{LinuxCredentialDeployment, LinuxRuntimeDeployment};

#[cfg(target_os = "linux")]
#[path = "provider_mcp_linux_child.rs"]
mod mcp_linux_child;

#[cfg(target_os = "linux")]
#[path = "provider_mcp_linux_owner.rs"]
mod mcp_owner;

#[cfg(target_os = "linux")]
#[path = "provider_mcp_linux_roster.rs"]
mod mcp_linux_roster;

#[cfg(target_os = "linux")]
#[path = "provider_mcp_linux_paths.rs"]
mod mcp_linux_paths;

#[cfg(target_os = "linux")]
#[path = "provider_egress_linux_resources.rs"]
pub(crate) mod linux_resources;

#[cfg(target_os = "linux")]
pub(crate) use mcp_owner::LinuxMcpDeployment;
