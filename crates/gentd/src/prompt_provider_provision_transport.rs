//! Capability-gated transport for one daemon-derived prompt-provider install confirmation.

use gent_protocol::{
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROMPT_PROVIDER_PROVISION_PROGRESS_CAPABILITY,
    PromptProviderProvisionFrame, PromptProviderProvisionProgressFrame, write_json_frame,
};
use gent_types::{CapabilitySet, ProviderInstallProgress};
use serde_json::Value;
use tokio::io::AsyncWrite;
use tokio::sync::watch;

use crate::{api::RuntimeApi, transport::write_error};

pub(crate) trait PromptProviderProvisionPort: Clone + Send + 'static {
    fn confirm(
        &self,
        request: PromptProviderProvisionFrame,
        progress: &dyn Fn(ProviderInstallProgress),
    ) -> Result<PromptProviderProvisionFrame, String>;
}

impl<R: RuntimeApi> PromptProviderProvisionPort for R {
    fn confirm(
        &self,
        request: PromptProviderProvisionFrame,
        progress: &dyn Fn(ProviderInstallProgress),
    ) -> Result<PromptProviderProvisionFrame, String> {
        self.prompt_provider_provision(request, progress)
    }
}

/// Dispatches one validated confirmation after explicit capability negotiation.
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
    P: PromptProviderProvisionPort,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == PROMPT_PROVIDER_PROVISION_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(request) = serde_json::from_value::<PromptProviderProvisionFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(request, PromptProviderProvisionFrame::Confirm { .. }) {
        write_error(
            stream,
            "invalidPromptProviderProvision",
            "prompt provider provision result frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    if request.validate().is_err() {
        write_error(
            stream,
            "invalidPromptProviderProvision",
            "prompt provider provision confirmation is invalid",
        )
        .await?;
        return Ok(true);
    }
    let port = port.clone();
    let confirmation = request.clone();
    let streaming = capabilities
        .0
        .iter()
        .any(|item| item == PROMPT_PROVIDER_PROVISION_PROGRESS_CAPABILITY);
    let (progress, mut updates) = watch::channel(None);
    let install = tokio::task::spawn_blocking(move || {
        port.confirm(confirmation, &|update| {
            if streaming {
                progress.send_replace(Some(update));
            }
        })
    });
    while streaming && updates.changed().await.is_ok() {
        let update = *updates.borrow_and_update();
        if let Some(frame) = update.and_then(|update| progress_frame(&request, update))
            && let Err(error) = write_json_frame(stream, &frame).await
        {
            let _ = install.await;
            return Err(error.into());
        }
    }
    match install.await? {
        Ok(reply) if reply.validate().is_ok() && correlated(&request, &reply) => {
            write_json_frame(stream, &reply).await?;
        }
        Ok(_) => {
            write_error(
                stream,
                "invalidPromptProviderProvision",
                "prompt provider provision runtime returned an uncorrelated response",
            )
            .await?;
        }
        Err(message) => write_error(stream, "promptProviderProvisionRejected", &message).await?,
    }
    Ok(true)
}

fn progress_frame(
    request: &PromptProviderProvisionFrame,
    progress: ProviderInstallProgress,
) -> Option<PromptProviderProvisionProgressFrame> {
    let PromptProviderProvisionFrame::Confirm {
        receipt_id,
        idempotency_key,
        host_epoch,
        prompt_receipt_id,
        conversation_id,
        run_id,
        ..
    } = request
    else {
        return None;
    };
    Some(
        PromptProviderProvisionProgressFrame::ProviderInstallProgress {
            receipt_id: receipt_id.clone(),
            idempotency_key: idempotency_key.clone(),
            host_epoch: *host_epoch,
            prompt_receipt_id: prompt_receipt_id.clone(),
            conversation_id: conversation_id.clone(),
            run_id: run_id.clone(),
            progress,
        },
    )
}

fn correlated(
    request: &PromptProviderProvisionFrame,
    reply: &PromptProviderProvisionFrame,
) -> bool {
    let PromptProviderProvisionFrame::Confirm {
        receipt_id,
        idempotency_key,
        host_epoch,
        prompt_receipt_id,
        conversation_id,
        run_id,
        ..
    } = request
    else {
        return false;
    };
    matches!(reply,
        PromptProviderProvisionFrame::Result {
            receipt,
            prompt_receipt_id: reply_prompt_receipt,
            conversation_id: reply_conversation,
            run_id: reply_run,
            ..
        } if receipt.receipt_id == *receipt_id
            && receipt.idempotency_key == *idempotency_key
            && receipt.host_epoch == *host_epoch
            && reply_prompt_receipt == prompt_receipt_id
            && reply_conversation == conversation_id
            && reply_run == run_id
    )
}

#[cfg(test)]
#[path = "prompt_provider_provision_transport_tests.rs"]
mod tests;
