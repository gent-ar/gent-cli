//! Capability-gated transport for a daemon-owned exact-run readiness decision.

use gent_protocol::{PROVIDER_READINESS_CAPABILITY, ProviderReadinessFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

pub(crate) trait ReadinessPort {
    fn assess(&self, request: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String>;
}

impl<R: RuntimeApi> ReadinessPort for R {
    fn assess(&self, request: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String> {
        self.provider_readiness(request)
    }
}

/// Dispatches one readiness assessment only after explicit capability negotiation.
pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    dispatch_port(stream, runtime, capabilities, raw).await
}

pub(crate) async fn dispatch_port<S, P>(
    stream: &mut S,
    port: &P,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    P: ReadinessPort,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == PROVIDER_READINESS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(request) = serde_json::from_value::<ProviderReadinessFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(request, ProviderReadinessFrame::Assess { .. }) {
        write_error(
            stream,
            "invalidProviderReadiness",
            "provider readiness response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match port.assess(request.clone()) {
        Ok(reply) if correlated(&request, &reply) => write_json_frame(stream, &reply).await?,
        Ok(_) => {
            write_error(
                stream,
                "invalidProviderReadiness",
                "provider readiness runtime returned an uncorrelated response",
            )
            .await?;
        }
        Err(message) => write_error(stream, "providerReadinessUnavailable", &message).await?,
    }
    Ok(true)
}

fn correlated(request: &ProviderReadinessFrame, reply: &ProviderReadinessFrame) -> bool {
    let ProviderReadinessFrame::Assess {
        conversation_id,
        run_id,
    } = request
    else {
        return false;
    };
    match reply {
        ProviderReadinessFrame::Ready {
            conversation_id: reply_conversation,
            run_id: reply_run,
            ..
        }
        | ProviderReadinessFrame::Review {
            conversation_id: reply_conversation,
            run_id: reply_run,
            ..
        }
        | ProviderReadinessFrame::Unavailable {
            conversation_id: reply_conversation,
            run_id: reply_run,
            ..
        } => reply_conversation == conversation_id && reply_run == run_id,
        ProviderReadinessFrame::Assess { .. } => false,
    }
}

#[cfg(test)]
#[path = "provider_readiness_transport_tests.rs"]
mod tests;
