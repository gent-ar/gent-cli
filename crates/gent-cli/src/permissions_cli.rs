//! Protocol-only terminal commands for durable permission preferences.

use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::{
    PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{PermissionCategory, PermissionMode, PolicyRecord, PolicyScope};
use serde_json::Value;
use std::path::PathBuf;

use crate::local_ipc::connect_and_negotiate;

pub(crate) mod agent_chat;
mod mode;
pub(crate) use mode::set_mode;

/// The daemon-owned local settings namespace; it is not a Git workspace selector.
const SETTINGS_WORKSPACE_ID: &str = "gent-local-settings";

#[derive(Debug, Subcommand)]
pub(crate) enum PermissionCommand {
    /// Print the current durable permission mode and approvals.
    Show,
    /// Append a new permission-policy revision. Existing approvals are replaced deliberately.
    Set(PermissionSetArgs),
    Respond(PermissionRespondArgs),
}

#[derive(Debug, Args)]
pub(crate) struct PermissionSetArgs {
    #[arg(long, value_enum)]
    pub(crate) mode: PermissionModeArgument,
    /// Approve one exact, provider-neutral tool name without widening a category.
    #[arg(long = "allow-tool")]
    pub(crate) allowed_tools: Vec<String>,
    /// Approve a complete typed category, such as `read` or `network`.
    #[arg(long = "allow-category", value_enum)]
    pub(crate) allowed_categories: Vec<PermissionCategoryArgument>,
    /// One-time confirmation required only when changing into the broad bypass mode.
    /// A persisted bypass policy applies to later normal `gent` and app connections.
    #[arg(long)]
    pub(crate) consent_bypass: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PermissionRespondArgs {
    #[arg(long)]
    pub(crate) response_json: String,
    #[arg(long)]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PermissionModeArgument {
    Default,
    Plan,
    AutoAcceptEdits,
    Autonomous,
    Bypass,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PermissionCategoryArgument {
    Read,
    Edit,
    Command,
    Network,
    Provider,
}

/// Executes one negotiated permission-policy command and returns the durable representation.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: PermissionCommand,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    match command {
        PermissionCommand::Show => Ok(serde_json::to_value(
            current(data_dir, no_autostart).await?,
        )?),
        PermissionCommand::Set(args) => Ok(serde_json::to_value(
            save(data_dir, no_autostart, args).await?,
        )?),
        PermissionCommand::Respond(args) => {
            agent_chat::respond_json(data_dir, no_autostart, args.response_json, args.receipt_id)
                .await
        }
    }
}

async fn save(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: PermissionSetArgs,
) -> Result<PolicyRecord, Box<dyn std::error::Error>> {
    let mode: PermissionMode = args.mode.into();
    if mode == PermissionMode::Bypass && !args.consent_bypass {
        return Err("changing to bypass mode requires --consent-bypass".into());
    }
    let current = current(data_dir.clone(), no_autostart).await?;
    let revision = current.as_ref().map_or(1, |policy| policy.revision + 1);
    let mut allowed_tools = args.allowed_tools;
    allowed_tools.sort();
    allowed_tools.dedup();
    let mut allowed_categories: Vec<PermissionCategory> = args
        .allowed_categories
        .into_iter()
        .map(Into::into)
        .collect();
    allowed_categories.sort();
    allowed_categories.dedup();
    let policy = policy(
        SETTINGS_WORKSPACE_ID,
        revision,
        mode,
        allowed_tools,
        allowed_categories,
    );
    exchange(
        data_dir,
        no_autostart,
        PermissionPolicyFrame::Save {
            request_id: uuid::Uuid::new_v4().to_string(),
            policy,
            bypass_consent: args.consent_bypass,
        },
    )
    .await
    .and_then(|frame| match frame {
        PermissionPolicyFrame::Saved { policy, .. } => Ok(policy),
        _ => Err("daemon did not save a permission policy".into()),
    })
}

async fn current(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<Option<PolicyRecord>, Box<dyn std::error::Error>> {
    current_for(data_dir, no_autostart, SETTINGS_WORKSPACE_ID.into()).await
}

pub(crate) async fn current_for(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
) -> Result<Option<PolicyRecord>, Box<dyn std::error::Error>> {
    exchange(
        data_dir,
        no_autostart,
        PermissionPolicyFrame::Current {
            request_id: uuid::Uuid::new_v4().to_string(),
            workspace_id,
        },
    )
    .await
    .and_then(|frame| match frame {
        PermissionPolicyFrame::CurrentPolicy { policy, .. } => Ok(policy),
        _ => Err("daemon did not return a permission policy".into()),
    })
}

pub(super) fn policy(
    workspace_id: &str,
    revision: u64,
    mode: PermissionMode,
    allowed_tools: Vec<String>,
    allowed_categories: Vec<PermissionCategory>,
) -> PolicyRecord {
    PolicyRecord {
        policy_id: format!("permission-policy-{}", uuid::Uuid::new_v4()),
        workspace_id: workspace_id.into(),
        scope: PolicyScope::ProviderPermissions,
        revision,
        mode,
        allowed_tools,
        allowed_categories,
    }
}

pub(super) async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: PermissionPolicyFrame,
) -> Result<PermissionPolicyFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == PERMISSION_POLICY_CAPABILITY)
    {
        return Err("gentd does not support permission policy; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return a permission policy frame")?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| {
            "daemon returned a permission policy response with a different request".into()
        })
}

fn valid_reply(request: &PermissionPolicyFrame, response: &PermissionPolicyFrame) -> bool {
    match (request, response) {
        (
            PermissionPolicyFrame::Current {
                request_id,
                workspace_id,
            },
            PermissionPolicyFrame::CurrentPolicy {
                request_id: reply,
                policy,
            },
        ) => {
            reply == request_id
                && policy.as_ref().is_none_or(|policy| {
                    policy.workspace_id == *workspace_id
                        && policy.scope == PolicyScope::ProviderPermissions
                })
        }
        (
            PermissionPolicyFrame::Save {
                request_id, policy, ..
            },
            PermissionPolicyFrame::Saved {
                request_id: reply,
                policy: saved,
            },
        ) => reply == request_id && saved == policy,
        _ => false,
    }
}

impl From<PermissionModeArgument> for PermissionMode {
    fn from(value: PermissionModeArgument) -> Self {
        match value {
            PermissionModeArgument::Default => Self::Default,
            PermissionModeArgument::Plan => Self::Plan,
            PermissionModeArgument::AutoAcceptEdits => Self::AutoAcceptEdits,
            PermissionModeArgument::Autonomous => Self::Autonomous,
            PermissionModeArgument::Bypass => Self::Bypass,
        }
    }
}

impl From<PermissionCategoryArgument> for PermissionCategory {
    fn from(value: PermissionCategoryArgument) -> Self {
        match value {
            PermissionCategoryArgument::Read => Self::Read,
            PermissionCategoryArgument::Edit => Self::Edit,
            PermissionCategoryArgument::Command => Self::Command,
            PermissionCategoryArgument::Network => Self::Network,
            PermissionCategoryArgument::Provider => Self::Provider,
        }
    }
}

#[cfg(test)]
#[path = "permissions_cli_tests.rs"]
mod tests;
