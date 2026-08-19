//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY, ATTACHMENTS_CAPABILITY,
    AgentChatIntentFrame, AgentChatTurnFollowFrame, AttachmentFrame, EVENT_STREAM_CAPABILITY,
    EventStreamFrame, WireFrame, negotiate, read_frame, read_json_frame, write_frame,
};
use gent_runtime::catalog::{RuntimeCapabilityProfile, declared_capabilities_with_profiles};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
use serde_json::Value;

use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use tokio::net::UnixListener;

use crate::api::RuntimeApi;

include!("transport_commands.rs");

/// Reports the capabilities backed by one explicitly composed runtime profile.
#[must_use]
pub(crate) fn observed_capabilities(profile: &RuntimeCapabilityProfile) -> CapabilitySet {
    declared_capabilities_with_profiles(profile)
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

/// Serves one connection until it completes or the daemon closes local IPC admissions.
///
/// The cancellation branch drops the in-flight protocol future, which also closes its stream.
/// It deliberately does not create a receipt or settle provider work: transport has no durable
/// authority and only stops accepting/serving client IPC.
pub(crate) async fn serve_connection_until<S, R>(
    stream: S,
    runtime: R,
    shutdown: crate::transport_shutdown::TransportShutdown,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: RuntimeApi,
{
    tokio::select! {
        result = serve_connection(stream, runtime) => result,
        () = shutdown.cancelled() => Ok(()),
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
                return crate::event_stream::serve(stream, runtime, after_cursor).await;
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
        if extensions.supports(AGENT_CHAT_TURN_FOLLOW_CAPABILITY) {
            if let Ok(AgentChatTurnFollowFrame::Follow {
                request_id,
                conversation_id,
                run_id,
                turn_id,
                after_cursor,
            }) = serde_json::from_value(raw.clone())
            {
                return crate::agent_chat_turn_follow::serve(
                    stream,
                    runtime,
                    request_id,
                    conversation_id,
                    run_id,
                    turn_id,
                    after_cursor,
                )
                .await;
            }
        }
        if dispatch_extension(&mut stream, &runtime, &extensions, &raw).await? {
            continue;
        }
        let frame = command_frame(&runtime, raw);
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
    if crate::orchestration_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
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
