//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, ATTACHMENTS_CAPABILITY, AgentChatIntentFrame,
    AttachmentFrame, CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY, EventStreamFrame, GOAL_CAPABILITY,
    WireFrame, negotiate, read_frame, read_json_frame, write_frame,
};
use gent_runtime::catalog::{RuntimeCapability, capability_set};
use gent_types::{CapabilitySet, EventResume, PROTOCOL_MAX, PROTOCOL_MIN};
use serde_json::Value;

#[cfg(unix)]
use gent_protocol::CONVERSATION_CONTENT_CAPABILITY;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use tokio::net::UnixListener;

use crate::api::RuntimeApi;

include!("transport_commands.rs");

/// Reports the capabilities backed by concrete post-handshake handlers in this adapter.
#[must_use]
pub(crate) fn observed_capabilities(
    agent_chat_enabled: bool,
    runtime_update_check_enabled: bool,
    runtime_maintenance_enabled: bool,
) -> CapabilitySet {
    let mut capabilities = capability_set([
        RuntimeCapability::Attachments,
        RuntimeCapability::Decisions,
        RuntimeCapability::EventResync,
        RuntimeCapability::EventStream,
        RuntimeCapability::Events,
        RuntimeCapability::HostEpoch,
        RuntimeCapability::PermissionPolicies,
        RuntimeCapability::Receipts,
    ]);
    capabilities
        .0
        .push(CONVERSATION_STATUS_CAPABILITY.to_owned());
    capabilities
        .0
        .push(CONVERSATION_INDEX_CAPABILITY.to_owned());
    capabilities
        .0
        .push(CONVERSATION_TIMELINE_CAPABILITY.to_owned());
    if agent_chat_enabled {
        capabilities
            .0
            .push(gent_protocol::AGENT_CHAT_INTENTS_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_CONVERSATIONS_CAPABILITY.to_owned());
        capabilities
            .0
            .push(AGENT_CHAT_TRANSCRIPT_CAPABILITY.to_owned());
        capabilities.0.push(GOAL_CAPABILITY.to_owned());
    }
    if runtime_update_check_enabled {
        capabilities
            .0
            .push(gent_protocol::RUNTIME_UPDATE_CHECK_CAPABILITY.to_owned());
    }
    if runtime_maintenance_enabled {
        capabilities
            .0
            .push(gent_protocol::RUNTIME_MAINTENANCE_CAPABILITY.to_owned());
    }
    #[cfg(unix)]
    capabilities
        .0
        .push(CONVERSATION_CONTENT_CAPABILITY.to_owned());
    capabilities
}

#[cfg(unix)]
pub async fn serve<R: RuntimeApi>(
    listener: UnixListener,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let (stream, _) = listener.accept().await?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(stream, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
    }
}

pub(crate) async fn serve_connection<S, R>(
    mut stream: S,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let Some(extensions) = establish_session(&mut stream, &runtime).await? else {
        return Ok(());
    };
    loop {
        let raw: Value = read_json_frame(&mut stream).await?;
        if extensions.supports(EVENT_STREAM_CAPABILITY) {
            if let Ok(EventStreamFrame::Attach { after_cursor }) =
                serde_json::from_value(raw.clone())
            {
                return crate::event_stream::serve(
                    stream,
                    runtime,
                    after_cursor,
                    extensions.supports("event-resync"),
                )
                .await;
            }
        }
        if extensions.supports(AGENT_CHAT_INTENTS_CAPABILITY) {
            if let Ok(AgentChatIntentFrame::Subscribe {
                request_id,
                conversation_id,
                after_cursor,
            }) = serde_json::from_value(raw.clone())
            {
                return crate::agent_chat_subscription::serve(
                    stream,
                    runtime,
                    request_id,
                    conversation_id,
                    after_cursor,
                )
                .await;
            }
        }
        if dispatch_extension(&mut stream, &runtime, &extensions, &raw).await? {
            continue;
        }
        let frame = command_frame(&runtime, raw, extensions.supports("event-resync"));
        match frame {
            Ok(frame) => write_frame(&mut stream, &frame).await?,
            Err(message) => write_error(&mut stream, "invalidCommand", &message).await?,
        }
    }
}

async fn establish_session<S, R>(
    mut stream: &mut S,
    runtime: &R,
) -> Result<Option<ExtensionSupport>, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let WireFrame::Hello(hello) = read_frame(&mut stream).await? else {
        write_error(
            &mut stream,
            "handshakeRequired",
            "hello must be the first frame",
        )
        .await?;
        return Ok(None);
    };
    let capabilities = match runtime.capabilities() {
        Ok(capabilities) => capabilities,
        Err(message) => {
            write_error(&mut stream, "capabilityUnavailable", &message).await?;
            return Ok(None);
        }
    };
    let extensions = match negotiate(&hello, PROTOCOL_MIN, PROTOCOL_MAX, &capabilities) {
        Ok(answer) => {
            let flags = ExtensionSupport(answer.capabilities.clone());
            write_frame(&mut stream, &WireFrame::Negotiated(answer)).await?;
            flags
        }
        Err(error) => {
            let message = error.to_string();
            write_error(&mut stream, "upgradeRequired", &message).await?;
            return Ok(None);
        }
    };
    Ok(Some(extensions))
}

async fn dispatch_extension<S, R>(
    stream: &mut S,
    runtime: &R,
    extensions: &ExtensionSupport,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if crate::agent_chat_read_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::agent_chat_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::permission_policy_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::provider_auth_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::reviewed_plan_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::goal_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::conversation_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::activity_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::runtime_update_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::runtime_maintenance_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if extensions.supports(ATTACHMENTS_CAPABILITY) {
        if let Ok(frame) = serde_json::from_value::<AttachmentFrame>(raw.clone()) {
            return crate::attachment_transport::write(stream, runtime, frame).await;
        }
    }
    Ok(false)
}

#[derive(Clone)]
struct ExtensionSupport(CapabilitySet);

impl ExtensionSupport {
    fn supports(&self, capability: &str) -> bool {
        self.0.0.iter().any(|item| item == capability)
    }
}

pub(crate) async fn write_error<S>(
    stream: &mut S,
    code: &str,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    write_frame(
        stream,
        &WireFrame::Error {
            code: code.into(),
            message: message.into(),
        },
    )
    .await?;
    Ok(())
}
