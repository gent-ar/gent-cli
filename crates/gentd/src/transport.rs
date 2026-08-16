//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{
    ATTACHMENTS_CAPABILITY, AttachmentFrame, CONVERSATION_INDEX_CAPABILITY,
    CONVERSATION_STATUS_CAPABILITY, CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY,
    EventStreamFrame, WireFrame, negotiate, read_frame, read_json_frame, write_frame,
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

/// Reports the capabilities backed by concrete post-handshake handlers in this adapter.
#[must_use]
pub(crate) fn observed_capabilities(agent_chat_enabled: bool) -> CapabilitySet {
    let mut capabilities = capability_set([
        RuntimeCapability::Attachments,
        RuntimeCapability::Decisions,
        RuntimeCapability::EventResync,
        RuntimeCapability::EventStream,
        RuntimeCapability::Events,
        RuntimeCapability::HostEpoch,
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
    if crate::agent_chat_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::conversation_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if crate::activity_transport::dispatch(stream, runtime, &extensions.0, raw).await? {
        return Ok(true);
    }
    if extensions.supports(ATTACHMENTS_CAPABILITY) {
        if let Ok(frame) = serde_json::from_value::<AttachmentFrame>(raw.clone()) {
            return crate::attachment_transport::write(stream, runtime, frame).await;
        }
    }
    Ok(false)
}

fn command_frame<R: RuntimeApi>(
    runtime: &R,
    raw: Value,
    client_supports_resync: bool,
) -> Result<WireFrame, String> {
    match serde_json::from_value::<WireFrame>(raw) {
        Ok(WireFrame::OnboardingRequest) => Ok(WireFrame::Onboarding(runtime.onboarding())),
        Ok(WireFrame::StatusRequest) => runtime.status().map(WireFrame::Status),
        Ok(WireFrame::DoctorRequest) => Ok(WireFrame::DoctorReport(runtime.doctor())),
        Ok(WireFrame::DependencyPlanRequest(request)) => {
            Ok(WireFrame::DependencyPlan(runtime.dependency_plan(request)))
        }
        Ok(WireFrame::DependencyActionRequest(request)) => runtime
            .dependency_action(request)
            .map(WireFrame::DependencyActionResult),
        Ok(WireFrame::DecisionSubmit(command)) => runtime
            .submit_decision(command)
            .map(WireFrame::DecisionSubmission),
        Ok(WireFrame::DecisionRecovery {
            decision_id,
            evidence,
        }) => runtime
            .apply_decision_recovery(decision_id, evidence)
            .map(WireFrame::DecisionSettlement),
        Ok(WireFrame::DecisionEvidence { .. }) => {
            Err("provider decision evidence is restricted to daemon-owned lifecycle inputs".into())
        }
        Ok(WireFrame::PublicRunStart(request)) => runtime
            .start_public_run(request)
            .map(WireFrame::PublicRunResponse),
        Ok(WireFrame::PublicRunResume(request)) => runtime
            .resume_public_run(request)
            .map(WireFrame::PublicRunResponse),
        Ok(WireFrame::PublicRunInterrupt(request)) => runtime
            .interrupt_public_run(request)
            .map(WireFrame::PublicRunResponse),
        Ok(WireFrame::Command(command)) => runtime.submit(command).map(WireFrame::Receipt),
        Ok(WireFrame::Subscribe { after_cursor }) => runtime
            .resume_events(after_cursor)
            .and_then(|resume| event_frame(resume, client_supports_resync)),
        Ok(_) | Err(_) => Err("frame is not valid after negotiation".into()),
    }
}

#[derive(Clone)]
struct ExtensionSupport(CapabilitySet);

impl ExtensionSupport {
    fn supports(&self, capability: &str) -> bool {
        self.0.0.iter().any(|item| item == capability)
    }
}

pub(crate) fn event_frame(
    resume: EventResume,
    client_supports_resync: bool,
) -> Result<WireFrame, String> {
    match resume {
        EventResume::Delta { events } => Ok(WireFrame::Events { events }),
        EventResume::Resync { snapshot, events } if client_supports_resync => {
            Ok(WireFrame::EventResync { snapshot, events })
        }
        EventResume::Resync { .. } => Err("event resync requires an upgraded client".into()),
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
