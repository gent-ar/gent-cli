//! Bounded client for the inert macOS provider-helper prepare protocol.
//!
//! This is intentionally a protocol edge, not a launcher. It has no process implementation and
//! does not implement [`crate::SandboxedProviderLaunch`]. A caller must supply a separately
//! reviewed transport; accepting a `Denied` result never authorizes a provider process.

use base64::{Engine, engine::general_purpose::STANDARD};
use gent_types::{SandboxNetworkPolicy, SandboxedLaunchRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BUNDLE_ID: &str = "io.gent.provider-helper";
const HELPER_VERSION: &str = "0.1.0";
const PROTOCOL_VERSION: u8 = 1;
const MAX_MESSAGE_BYTES: usize = 32 * 1024;

/// The only transport surface accepted by the helper client.
///
/// Implementations must exchange exactly one JSON request and response. They must bound both
/// sides and must not fall back to a shell, `PATH`, or an alternate provider launcher.
pub trait MacosProviderHelperTransport: Send + Sync {
    /// Transport-specific failure retained only at this private protocol edge.
    type Error: Send + Sync + 'static;

    /// Exchanges the already bounded protocol message with the exact reviewed helper.
    ///
    /// # Errors
    /// Returns the transport-specific error when the helper exchange cannot complete.
    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, Self::Error>;
}

/// Input which is not part of a provider command or provider-native state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacosHelperPrepare {
    pub request_id: String,
    pub workspace_bookmark: Option<String>,
}

/// The helper's only current successful parse outcome: a fail-closed denial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacosHelperDenial {
    WorkspaceBookmarkRequired,
    WorkspaceBookmarkInvalid,
    WorkspaceAuthorizationDenied,
    ContainmentSemanticsUnavailable,
    HelperIdentityInvalid,
}

/// Failure while constructing or validating a macOS helper protocol exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MacosProviderHelperError {
    #[error("helper request id is invalid")]
    InvalidRequestId,
    #[error("workspace bookmark is invalid")]
    InvalidWorkspaceBookmark,
    #[error("helper request exceeded its byte limit")]
    RequestTooLarge,
    #[error("helper transport is unavailable")]
    TransportUnavailable,
    #[error("helper response exceeded its byte limit")]
    ResponseTooLarge,
    #[error("helper response has an invalid protocol shape")]
    InvalidResponse,
    #[error("helper identity did not match the reviewed bundle")]
    IdentityMismatch,
    #[error("helper response was for another request")]
    RequestMismatch,
    #[error("helper returned an unknown denial")]
    UnknownDenial,
}

/// Typed protocol client for one exact helper bundle/version.
#[derive(Clone, Debug)]
pub struct MacosProviderHelperClient<T> {
    transport: T,
}

impl<T> MacosProviderHelperClient<T>
where
    T: MacosProviderHelperTransport,
{
    /// Creates a protocol-only client. This does not inspect, spawn, or authorize a provider.
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Validates an inert helper `prepare` exchange and returns its explicit denial.
    ///
    /// The current helper deliberately cannot report enforcement or a launched process. Any
    /// unexpected success-shaped response is rejected rather than interpreted as authorization.
    ///
    /// # Errors
    /// Returns an error when either protocol message is unsafe, unavailable, or fails exact
    /// identity and denial-shape validation.
    pub fn prepare(
        &self,
        request: &SandboxedLaunchRequest,
        input: &MacosHelperPrepare,
    ) -> Result<MacosHelperDenial, MacosProviderHelperError> {
        validate_input(input)?;
        let payload = serde_json::to_vec(&WireRequest::from_request(request, input))
            .map_err(|_| MacosProviderHelperError::InvalidResponse)?;
        if payload.len() > MAX_MESSAGE_BYTES {
            return Err(MacosProviderHelperError::RequestTooLarge);
        }
        let response = self
            .transport
            .exchange(&payload)
            .map_err(|_| MacosProviderHelperError::TransportUnavailable)?;
        if response.len() > MAX_MESSAGE_BYTES {
            return Err(MacosProviderHelperError::ResponseTooLarge);
        }
        decode_response(&response, &input.request_id)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRequest<'a> {
    protocol_version: u8,
    request_id: &'a str,
    operation: &'static str,
    helper: HelperIdentity<'static>,
    provider: ProviderLock<'a>,
    profile: LaunchProfile<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HelperIdentity<'a> {
    bundle_id: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderLock<'a> {
    name: &'a str,
    canonical_path: &'a str,
    file_identity: &'a str,
    digest_sha256: &'a str,
    version: &'a str,
    compatibility_entry: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchProfile<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_bookmark: Option<&'a str>,
    profile_digest_sha256: String,
    network: Network<'a>,
    limits: Limits,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Network<'a> {
    mode: &'static str,
    egress_policy_digest_sha256: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    #[serde(rename = "maxProcesses")]
    processes: u16,
    #[serde(rename = "maxMemoryBytes")]
    memory_bytes: u64,
    #[serde(rename = "maxCpuTimeMs")]
    cpu_time_ms: u64,
}

impl<'a> WireRequest<'a> {
    fn from_request(request: &'a SandboxedLaunchRequest, input: &'a MacosHelperPrepare) -> Self {
        let profile = &request.profile;
        let (mode, egress_policy_digest_sha256) = match profile.network() {
            SandboxNetworkPolicy::Disabled => ("disabled", None),
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256,
            } => ("reviewedEgress", Some(policy_digest_sha256.as_str())),
        };
        let limits = profile.limits();
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: &input.request_id,
            operation: "prepare",
            helper: HelperIdentity {
                bundle_id: BUNDLE_ID,
                version: HELPER_VERSION,
            },
            provider: ProviderLock {
                name: &request.lock.provider,
                canonical_path: &request.lock.canonical_path,
                file_identity: &request.lock.file_identity,
                digest_sha256: &request.lock.digest_sha256,
                version: &request.lock.version,
                compatibility_entry: &request.lock.compatibility_entry,
            },
            profile: LaunchProfile {
                workspace_bookmark: input.workspace_bookmark.as_deref(),
                profile_digest_sha256: profile.digest_sha256(),
                network: Network {
                    mode,
                    egress_policy_digest_sha256,
                },
                limits: Limits {
                    processes: limits.max_processes,
                    memory_bytes: limits.max_memory_bytes,
                    cpu_time_ms: limits.max_cpu_time_ms,
                },
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireResponse {
    protocol_version: u8,
    request_id: String,
    helper: ResponseIdentity,
    result: WireResult,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseIdentity {
    bundle_id: String,
    version: String,
}

#[derive(Deserialize)]
struct WireResult {
    state: String,
    reason: String,
}

fn validate_input(input: &MacosHelperPrepare) -> Result<(), MacosProviderHelperError> {
    if input.request_id.is_empty() || input.request_id.len() > 96 || input.request_id.contains('\0')
    {
        return Err(MacosProviderHelperError::InvalidRequestId);
    }
    if let Some(bookmark) = &input.workspace_bookmark {
        if bookmark.len() > 8192 || STANDARD.decode(bookmark).is_err() {
            return Err(MacosProviderHelperError::InvalidWorkspaceBookmark);
        }
    }
    Ok(())
}

fn decode_response(
    response: &[u8],
    expected_id: &str,
) -> Result<MacosHelperDenial, MacosProviderHelperError> {
    let value: Value =
        serde_json::from_slice(response).map_err(|_| MacosProviderHelperError::InvalidResponse)?;
    exact_keys(
        &value,
        &["protocolVersion", "requestId", "helper", "result"],
    )?;
    exact_keys(&value["helper"], &["bundleId", "version"])?;
    exact_keys(&value["result"], &["state", "reason"])?;
    let response: WireResponse =
        serde_json::from_value(value).map_err(|_| MacosProviderHelperError::InvalidResponse)?;
    if response.protocol_version != PROTOCOL_VERSION
        || response.helper.bundle_id != BUNDLE_ID
        || response.helper.version != HELPER_VERSION
    {
        return Err(MacosProviderHelperError::IdentityMismatch);
    }
    if response.request_id != expected_id {
        return Err(MacosProviderHelperError::RequestMismatch);
    }
    if response.result.state != "denied" {
        return Err(MacosProviderHelperError::InvalidResponse);
    }
    match response.result.reason.as_str() {
        "workspaceBookmarkRequired" => Ok(MacosHelperDenial::WorkspaceBookmarkRequired),
        "workspaceBookmarkInvalid" => Ok(MacosHelperDenial::WorkspaceBookmarkInvalid),
        "workspaceAuthorizationDenied" => Ok(MacosHelperDenial::WorkspaceAuthorizationDenied),
        "containmentSemanticsUnavailable" => Ok(MacosHelperDenial::ContainmentSemanticsUnavailable),
        "helperIdentityInvalid" => Ok(MacosHelperDenial::HelperIdentityInvalid),
        _ => Err(MacosProviderHelperError::UnknownDenial),
    }
}

fn exact_keys(value: &Value, expected: &[&str]) -> Result<(), MacosProviderHelperError> {
    let object = value
        .as_object()
        .ok_or(MacosProviderHelperError::InvalidResponse)?;
    (object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key)))
        .then_some(())
        .ok_or(MacosProviderHelperError::InvalidResponse)
}

#[cfg(test)]
#[path = "macos_provider_helper_tests.rs"]
mod tests;
