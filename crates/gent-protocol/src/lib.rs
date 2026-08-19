//! Versioned wire DTOs and length-prefixed JSON framing shared by every transport.

use gent_types::{
    CapabilitySet, Command, DecisionCommand, DecisionSettlement, DoctorReport, EventPage,
    HostStatus, OnboardingState, Receipt,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

mod agent_chat;
mod agent_chat_intent;
mod attachments;
mod conversation_activity;
mod conversation_content;
mod conversation_index;
mod conversation_status;
mod conversation_timeline;
mod decision;
mod dependencies;
mod event_stream;
mod external_provider_bridge;
mod goal;
mod orchestration;
mod permission_policy;
mod provider_auth;
mod reviewed_plan;
mod runs;
mod runtime_maintenance;
mod runtime_update;
mod turn_follow;

pub use agent_chat::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AgentChatConversationFrame, AgentChatTranscriptFrame,
};
pub use agent_chat_intent::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, AgentChatSubscriptionEnd,
};
pub use attachments::{ATTACHMENTS_CAPABILITY, AttachmentFrame};
pub use conversation_activity::{CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame};
pub use conversation_content::{
    CONVERSATION_CONTENT_CAPABILITY, ContentPageError, ConversationContentFrame,
    MAX_CONVERSATION_CONTENT_PAGE_BYTES, bound_content_page,
};
pub use conversation_index::{CONVERSATION_INDEX_CAPABILITY, ConversationIndexFrame};
pub use conversation_status::{CONVERSATION_STATUS_CAPABILITY, ConversationStatusFrame};
pub use conversation_timeline::{CONVERSATION_TIMELINE_CAPABILITY, ConversationTimelineFrame};
pub use decision::{
    DecisionEvidence, DecisionRecoveryEvidence, DecisionSubmission, ProviderDecisionEvidence,
};
pub use dependencies::{
    DependencyAction, DependencyActionRequest, DependencyActionResult, DependencyActionState,
    DependencyPlan, DependencyPlanRequest, DependencyProvider, dependency_plan_digest,
};
pub use event_stream::{EVENT_STREAM_CAPABILITY, EventStreamFrame};
pub use external_provider_bridge::{
    EXTERNAL_PROVIDER_BRIDGE_CAPABILITY, ExternalProviderBridgeFrame, ExternalProviderBridgeHello,
    ExternalProviderBridgeNegotiated,
};
pub use goal::{GOAL_CAPABILITY, GoalFrame, GoalFrameError, MAX_GOAL_FRAME_BYTES};
pub use orchestration::{
    MAX_ORCHESTRATION_FRAME_BYTES, ORCHESTRATION_CAPABILITY, OrchestrationFrame,
    OrchestrationFrameError,
};
pub use permission_policy::{PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame};
pub use provider_auth::{
    MAX_PROVIDER_AUTH_FRAME_BYTES, PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame,
    ProviderAuthFrameError, read_provider_auth_frame, write_provider_auth_frame,
};
pub use reviewed_plan::{REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame, ReviewedPlanFrameError};
pub use runs::{
    PublicRunInterruptRequest, PublicRunOutcome, PublicRunResponse, PublicRunResumeRequest,
    PublicRunStartRequest,
};
pub use runtime_maintenance::{RUNTIME_MAINTENANCE_CAPABILITY, RuntimeMaintenanceFrame};
pub use runtime_update::{RUNTIME_UPDATE_CHECK_CAPABILITY, RuntimeUpdateCheckFrame};
pub use turn_follow::{
    AGENT_CHAT_TURN_FOLLOW_CAPABILITY, AgentChatTurnFollowEnd, AgentChatTurnFollowFrame,
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
    OnboardingRequest,
    Onboarding(OnboardingState),
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
    /// Compatibility tombstone for provider evidence from older public clients.
    ///
    /// The daemon deliberately rejects this frame: provider acknowledgement is lifecycle-owned.
    DecisionRecovery {
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
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
        page: EventPage,
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
    write_json_frame(writer, frame).await
}

/// Encodes and writes one bounded length-prefixed JSON value.
///
/// This is the generic framing primitive for additive local protocol endpoints. [`WireFrame`]
/// retains its stable command/receipt/event contract through [`write_frame`].
///
/// # Errors
/// Returns an I/O error when serialization or writing fails.
pub async fn write_json_frame<W, T>(writer: &mut W, frame: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
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
    read_json_frame(reader).await
}

/// Reads and decodes one bounded length-prefixed JSON value.
///
/// # Errors
/// Returns an I/O error for malformed, oversized, or incomplete frames.
pub async fn read_json_frame<R, T>(reader: &mut R) -> io::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
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
