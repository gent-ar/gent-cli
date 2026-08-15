//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{
    CONVERSATION_STATUS_CAPABILITY, ConversationStatusFrame, WireFrame, negotiate, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_runtime::catalog::{RuntimeCapability, capability_set};
use gent_types::{CapabilitySet, EventResume, PROTOCOL_MAX, PROTOCOL_MIN};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use tokio::net::UnixListener;

use crate::api::RuntimeApi;

/// Reports the capabilities backed by concrete post-handshake handlers in this adapter.
#[must_use]
pub(crate) fn observed_capabilities() -> CapabilitySet {
    let mut capabilities = capability_set([
        RuntimeCapability::Decisions,
        RuntimeCapability::EventResync,
        RuntimeCapability::Events,
        RuntimeCapability::HostEpoch,
        RuntimeCapability::Receipts,
    ]);
    capabilities
        .0
        .push(CONVERSATION_STATUS_CAPABILITY.to_owned());
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
    let WireFrame::Hello(hello) = read_frame(&mut stream).await? else {
        return write_error(
            &mut stream,
            "handshakeRequired",
            "hello must be the first frame",
        )
        .await;
    };
    let capabilities = match runtime.capabilities() {
        Ok(capabilities) => capabilities,
        Err(message) => return write_error(&mut stream, "capabilityUnavailable", &message).await,
    };
    let (client_supports_resync, client_supports_conversation_status) =
        match negotiate(&hello, PROTOCOL_MIN, PROTOCOL_MAX, &capabilities) {
            Ok(answer) => {
                let supported =
                    |capability| answer.capabilities.0.iter().any(|item| item == capability);
                let flags = (
                    supported("event-resync"),
                    supported(CONVERSATION_STATUS_CAPABILITY),
                );
                write_frame(&mut stream, &WireFrame::Negotiated(answer)).await?;
                flags
            }
            Err(error) => {
                return write_error(&mut stream, "upgradeRequired", &error.to_string()).await;
            }
        };
    loop {
        let raw: Value = read_json_frame(&mut stream).await?;
        if client_supports_conversation_status {
            if let Ok(ConversationStatusFrame::Request { conversation_id }) =
                serde_json::from_value(raw.clone())
            {
                match runtime.conversation_status(&conversation_id) {
                    Ok(status) => {
                        write_json_frame(&mut stream, &ConversationStatusFrame::Status(status))
                            .await?;
                    }
                    Err(message) => write_error(&mut stream, "invalidRequest", &message).await?,
                }
                continue;
            }
        }
        let frame = match serde_json::from_value::<WireFrame>(raw) {
            Ok(WireFrame::StatusRequest) => runtime.status().map(WireFrame::Status),
            Ok(WireFrame::DoctorRequest) => Ok(WireFrame::DoctorReport(runtime.doctor())),
            Ok(WireFrame::DependencyPlanRequest(request)) => {
                Ok(WireFrame::DependencyPlan(runtime.dependency_plan(request)))
            }
            Ok(WireFrame::DependencyActionRequest(request)) => Ok(
                WireFrame::DependencyActionResult(runtime.dependency_action(request)),
            ),
            Ok(WireFrame::DecisionSubmit(command)) => runtime
                .submit_decision(command)
                .map(WireFrame::DecisionSubmission),
            Ok(WireFrame::DecisionEvidence {
                decision_id,
                evidence,
            }) => runtime
                .apply_decision_evidence(decision_id, evidence)
                .map(WireFrame::DecisionSettlement),
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
        };
        match frame {
            Ok(frame) => write_frame(&mut stream, &frame).await?,
            Err(message) => write_error(&mut stream, "invalidCommand", &message).await?,
        }
    }
}

fn event_frame(resume: EventResume, client_supports_resync: bool) -> Result<WireFrame, String> {
    match resume {
        EventResume::Delta { events } => Ok(WireFrame::Events { events }),
        EventResume::Resync { snapshot, events } if client_supports_resync => {
            Ok(WireFrame::EventResync { snapshot, events })
        }
        EventResume::Resync { .. } => Err("event resync requires an upgraded client".into()),
    }
}

async fn write_error<S>(
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

#[cfg(test)]
mod tests {
    use gent_types::{EventResume, EventSnapshot, HostEpoch};
    use serde_json::json;

    use super::event_frame;

    #[test]
    fn stale_event_feeds_require_the_explicit_resync_capability() {
        let resume = EventResume::Resync {
            snapshot: EventSnapshot {
                cursor: 4,
                host_epoch: HostEpoch(1),
                schema_version: 1,
                payload: json!({ "safe": true }),
            },
            events: Vec::new(),
        };
        assert!(event_frame(resume.clone(), false).is_err());
        assert!(matches!(
            event_frame(resume, true),
            Ok(gent_protocol::WireFrame::EventResync { .. })
        ));
    }
}
