//! Terminal exchanges for daemon-owned provider readiness and exact install consent.
//!
//! The terminal only correlates an already accepted prompt. Gentd derives the selected provider,
//! package, policy, installer command, and review artifact from its durable ledger.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use gent_protocol::{
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_READINESS_CAPABILITY,
    PromptProviderProvisionFrame, ProviderReadinessFrame, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRunId, HostEpoch, ReceiptId};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::local_ipc::connect_and_negotiate;

/// Public terminal operations that may unblock one already-persisted prompt.
#[derive(Debug, Subcommand)]
pub(crate) enum ProviderLifecycleCommand {
    /// Ask Gentd whether the exact current run can start, or obtain its daemon-issued review.
    Readiness(ReadinessArgs),
    /// Confirm or reject the exact review Gentd issued for an already accepted prompt.
    Provision(ProvisionArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ReadinessArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    run_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct ProvisionArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    run_id: String,
    /// Receipt returned when the held prompt was accepted.
    #[arg(long)]
    prompt_receipt_id: String,
    /// Exact digest returned by `gent provider readiness` for this held prompt.
    #[arg(long)]
    reviewed_plan_digest: String,
    /// Explicitly permit the daemon-owned, reviewed install. Omit to reject it.
    #[arg(long)]
    consent: bool,
    /// Reuse this key only to retry the same consent after interruption.
    #[arg(long)]
    idempotency_key: Option<String>,
}

/// Executes one exact provider lifecycle operation without constructing provider work.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: ProviderLifecycleCommand,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    match command {
        ProviderLifecycleCommand::Readiness(args) => {
            let response =
                readiness(data_dir, no_autostart, &args.conversation_id, &args.run_id).await?;
            Ok(serde_json::to_value(response)?)
        }
        ProviderLifecycleCommand::Provision(args) => {
            let response = provision(data_dir, no_autostart, args).await?;
            Ok(serde_json::to_value(response)?)
        }
    }
}

async fn readiness(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    run_id: &str,
) -> Result<ProviderReadinessFrame, Box<dyn std::error::Error>> {
    let request = ProviderReadinessFrame::Assess {
        conversation_id: AgentChatConversationId(conversation_id.into()),
        run_id: AgentChatRunId(run_id.into()),
    };
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(
        &capabilities,
        PROVIDER_READINESS_CAPABILITY,
        "provider readiness",
    )?;
    write_json_frame(&mut stream, &request).await?;
    let response = read_json_or_error(&mut stream, "provider readiness").await?;
    readiness_reply_matches(&request, &response)
        .then_some(response)
        .ok_or_else(|| "daemon returned an uncorrelated provider readiness response".into())
}

async fn provision(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: ProvisionArgs,
) -> Result<PromptProviderProvisionFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(
        &capabilities,
        PROMPT_PROVIDER_PROVISION_CAPABILITY,
        "prompt provider provisioning",
    )?;
    let host_epoch = host_epoch(&mut stream).await?;
    let request = provision_request(args, host_epoch)?;
    write_json_frame(&mut stream, &request).await?;
    let response = read_json_or_error(&mut stream, "prompt provider provision").await?;
    provision_reply_matches(&request, &response)
        .then_some(response)
        .ok_or_else(|| "daemon returned an uncorrelated prompt provider provision response".into())
}

fn provision_request(
    args: ProvisionArgs,
    host_epoch: HostEpoch,
) -> Result<PromptProviderProvisionFrame, Box<dyn std::error::Error>> {
    let (receipt_id, idempotency_key) = provision_identifiers(args.idempotency_key);
    let request = PromptProviderProvisionFrame::Confirm {
        receipt_id,
        idempotency_key,
        host_epoch,
        prompt_receipt_id: ReceiptId(args.prompt_receipt_id),
        conversation_id: AgentChatConversationId(args.conversation_id),
        run_id: AgentChatRunId(args.run_id),
        consent_granted: args.consent,
        reviewed_plan_digest: args.reviewed_plan_digest,
    };
    request.validate()?;
    Ok(request)
}

fn provision_identifiers(idempotency_key: Option<String>) -> (ReceiptId, String) {
    let Some(idempotency_key) = idempotency_key else {
        return (ReceiptId::new(), ReceiptId::new().0);
    };
    let mut hasher = Sha256::new();
    hasher.update(b"gent.prompt-provider-provision.receipt.v1\0");
    hasher.update(idempotency_key.as_bytes());
    let receipt_id = ReceiptId(format!("prompt-provider-provision-{:x}", hasher.finalize()));
    (receipt_id, idempotency_key)
}

fn require_capability(
    capabilities: &gent_types::CapabilitySet,
    required: &str,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    capabilities
        .0
        .iter()
        .any(|capability| capability == required)
        .then_some(())
        .ok_or_else(|| {
            format!(
                "{operation} is unavailable while gentd runs in observer mode; no provider work was started"
            )
            .into()
        })
}

async fn host_epoch(
    stream: &mut crate::local_ipc::LocalStream,
) -> Result<HostEpoch, Box<dyn std::error::Error>> {
    write_frame(stream, &WireFrame::StatusRequest).await?;
    match read_frame(stream).await? {
        WireFrame::Status(status) => Ok(status.host_epoch),
        WireFrame::Error { message, .. } => Err(message.into()),
        _ => Err("daemon did not return host status before prompt provider provision".into()),
    }
}

async fn read_json_or_error<T>(
    stream: &mut crate::local_ipc::LocalStream,
    operation: &str,
) -> Result<T, Box<dyn std::error::Error>>
where
    T: serde::de::DeserializeOwned,
{
    let raw: Value = read_json_frame(stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    serde_json::from_value(raw)
        .map_err(|_| format!("daemon did not return a {operation} response").into())
}

fn readiness_reply_matches(
    request: &ProviderReadinessFrame,
    response: &ProviderReadinessFrame,
) -> bool {
    let ProviderReadinessFrame::Assess {
        conversation_id,
        run_id,
    } = request
    else {
        return false;
    };
    matches!(response,
        ProviderReadinessFrame::Ready { conversation_id: reply_conversation, run_id: reply_run, .. }
        | ProviderReadinessFrame::Review { conversation_id: reply_conversation, run_id: reply_run, .. }
        | ProviderReadinessFrame::Unavailable { conversation_id: reply_conversation, run_id: reply_run, .. }
            if reply_conversation == conversation_id && reply_run == run_id
    )
}

fn provision_reply_matches(
    request: &PromptProviderProvisionFrame,
    response: &PromptProviderProvisionFrame,
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
    matches!(response,
        PromptProviderProvisionFrame::Result {
            receipt, prompt_receipt_id: reply_prompt, conversation_id: reply_conversation, run_id: reply_run, ..
        } if receipt.receipt_id == *receipt_id
            && receipt.idempotency_key == *idempotency_key
            && receipt.host_epoch == *host_epoch
            && reply_prompt == prompt_receipt_id
            && reply_conversation == conversation_id
            && reply_run == run_id
    )
}

#[cfg(all(test, unix))]
#[path = "provider_lifecycle_cli_tests.rs"]
mod tests;
