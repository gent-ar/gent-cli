use gent_protocol::{
    runtime_owner::{RUNTIME_OWNER_CONTROL_CAPABILITY, RuntimeOwnerFrame},
    write_json_frame,
};
use serde_json::Value;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::{ExtensionSupport, write_error};
use crate::api::RuntimeApi;

pub(super) async fn dispatch<S: AsyncWrite + Unpin, R: RuntimeApi>(
    stream: &mut S,
    runtime: &R,
    extensions: &ExtensionSupport,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if !extensions.supports(RUNTIME_OWNER_CONTROL_CAPABILITY)
        && !extensions.supports(gent_protocol::runtime_owner::RUNTIME_OWNER_IDENTITY_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<RuntimeOwnerFrame>(raw.clone()) else {
        return Ok(false);
    };
    let identity_request = matches!(&frame, RuntimeOwnerFrame::RuntimeOwnerIdentity { .. });
    if (identity_request
        && !extensions.supports(gent_protocol::runtime_owner::RUNTIME_OWNER_IDENTITY_CAPABILITY))
        || (!identity_request && !extensions.supports(RUNTIME_OWNER_CONTROL_CAPABILITY))
    {
        return Ok(false);
    }
    let Some(owner) = runtime.runtime_owner_control_port() else {
        write_error(
            stream,
            "runtimeOwnerUnavailable",
            "runtime owner control is unavailable",
        )
        .await?;
        return Ok(true);
    };
    let request_id = match &frame {
        RuntimeOwnerFrame::QuiesceRuntimeOwner { request_id, .. } => Some(request_id.clone()),
        _ => None,
    };
    match owner.exchange_typed(frame) {
        Ok(response) => {
            let quiesced = matches!(response, RuntimeOwnerFrame::RuntimeOwnerQuiesced { .. });
            write_json_frame(stream, &response).await?;
            stream.flush().await?;
            if quiesced {
                owner.acknowledge_quiesce();
            }
        }
        Err(failure) => {
            if let Some(request_id) = request_id.filter(|_| {
                extensions.supports(
                    gent_protocol::runtime_owner::RUNTIME_OWNER_QUIESCE_REJECTION_CAPABILITY,
                )
            }) {
                write_json_frame(
                    stream,
                    &RuntimeOwnerFrame::RuntimeOwnerQuiesceRejected {
                        request_id,
                        failure,
                    },
                )
                .await?;
            } else {
                write_error(stream, "runtimeOwnerQuiesceRejected", &failure.message).await?;
            }
        }
    }
    Ok(true)
}
