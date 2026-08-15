//! Versioned wire DTOs and length-prefixed JSON framing shared by every transport.

use std::io;
use std::str::FromStr;

use gent_types::{
    CapabilitySet, Command, DecisionCommand, DecisionSettlement, DoctorReport, Event, HostStatus,
    Receipt,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

mod decision;
mod runs;

pub use decision::{DecisionEvidence, DecisionSubmission};
pub use runs::{
    PublicRunInterruptRequest, PublicRunOutcome, PublicRunResponse, PublicRunResumeRequest,
    PublicRunStartRequest,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol_min: u16,
    pub protocol_max: u16,
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Negotiated {
    pub protocol: u16,
    pub capabilities: CapabilitySet,
}

/// A publicly installable provider. Private bridges are intentionally excluded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyProvider {
    Claude,
    Codex,
}

impl DependencyProvider {
    /// Returns the stable public provider identifier used in durable locks.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

impl FromStr for DependencyProvider {
    type Err = ProtocolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err(ProtocolError::UnsupportedProvider(value.into())),
        }
    }
}

/// An explicit action a user may request for a public provider dependency.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyAction {
    Install,
    Update,
}

impl FromStr for DependencyAction {
    type Err = ProtocolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "install" => Ok(Self::Install),
            "update" => Ok(Self::Update),
            _ => Err(ProtocolError::UnsupportedDependencyAction(value.into())),
        }
    }
}

/// A read-only request for a dependency action plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPlanRequest {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
}

/// An explicit confirmation to act on a prior plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyActionRequest {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
    pub consent_granted: bool,
}

/// Human-readable, vendor-directed plan. Gent never embeds a provider installer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPlan {
    pub provider: DependencyProvider,
    pub action: DependencyAction,
    pub instruction: String,
    pub consent_required: bool,
}

/// Result of evaluating an explicit dependency action request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyActionState {
    ConsentRequired,
    InstallerNotConfigured,
}

/// The daemon's non-mutating dependency-action result for this milestone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyActionResult {
    pub plan: DependencyPlan,
    pub state: DependencyActionState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum WireFrame {
    Hello(Hello),
    Negotiated(Negotiated),
    Command(Command),
    Receipt(Receipt),
    StatusRequest,
    Status(HostStatus),
    DoctorRequest,
    DoctorReport(DoctorReport),
    DependencyPlanRequest(DependencyPlanRequest),
    DependencyPlan(DependencyPlan),
    DependencyActionRequest(DependencyActionRequest),
    DependencyActionResult(DependencyActionResult),
    DecisionSubmit(DecisionCommand),
    DecisionSubmission(DecisionSubmission),
    DecisionEvidence {
        decision_id: String,
        evidence: DecisionEvidence,
    },
    DecisionSettlement(DecisionSettlement),
    PublicRunStart(PublicRunStartRequest),
    PublicRunResume(PublicRunResumeRequest),
    PublicRunInterrupt(PublicRunInterruptRequest),
    PublicRunResponse(PublicRunResponse),
    Subscribe {
        after_cursor: u64,
    },
    Events {
        events: Vec<Event>,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unsupported public provider: {0}")]
    UnsupportedProvider(String),
    #[error("unsupported dependency action: {0}")]
    UnsupportedDependencyAction(String),
    #[error(
        "protocol ranges do not overlap: client {client_min}..={client_max}, server {server_min}..={server_max}"
    )]
    IncompatibleVersion {
        client_min: u16,
        client_max: u16,
        server_min: u16,
        server_max: u16,
    },
}

/// Negotiates a shared protocol version and capability intersection.
///
/// # Errors
/// Returns [`ProtocolError::IncompatibleVersion`] when ranges do not overlap.
pub fn negotiate(
    hello: &Hello,
    server_min: u16,
    server_max: u16,
    server_capabilities: &CapabilitySet,
) -> Result<Negotiated, ProtocolError> {
    let minimum = hello.protocol_min.max(server_min);
    let maximum = hello.protocol_max.min(server_max);
    if minimum > maximum {
        return Err(ProtocolError::IncompatibleVersion {
            client_min: hello.protocol_min,
            client_max: hello.protocol_max,
            server_min,
            server_max,
        });
    }
    Ok(Negotiated {
        protocol: maximum,
        capabilities: hello.capabilities.intersection(server_capabilities),
    })
}

/// Encodes and writes one bounded length-prefixed JSON frame.
///
/// # Errors
/// Returns an I/O error when serialization or writing fails.
pub async fn write_frame<W>(writer: &mut W, frame: &WireFrame) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(frame).map_err(io::Error::other)?;
    let length = u32::try_from(body.len()).map_err(|_| io::Error::other("frame too large"))?;
    writer.write_u32(length).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// Reads and decodes one bounded length-prefixed JSON frame.
///
/// # Errors
/// Returns an I/O error for malformed, oversized, or incomplete frames.
pub async fn read_frame<R>(reader: &mut R) -> io::Result<WireFrame>
where
    R: AsyncRead + Unpin,
{
    let length = reader.read_u32().await?;
    let length = usize::try_from(length).map_err(|_| io::Error::other("invalid frame length"))?;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds maximum size",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
