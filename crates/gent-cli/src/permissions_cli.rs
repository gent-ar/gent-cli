use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::{
    PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame, read_json_frame, write_json_frame,
};
use gent_types::{PermissionCategory, PermissionMode, PolicyRecord, PolicyScope};
use serde_json::Value;
use std::path::PathBuf;

use crate::cli_error::{CliError, Failure};
use crate::local_ipc::connect_and_negotiate;

pub(crate) mod agent_chat;
mod conversions;
mod mode;
mod workspace;
use conversions::valid_reply;
pub(crate) use mode::set_mode;

#[derive(Debug, Subcommand)]
pub(crate) enum PermissionCommand {
    #[command(about = "Print the permission policy that gates chats in a workspace")]
    Show(PermissionShowArgs),
    #[command(about = "Save a new permission-policy revision for a workspace")]
    Set(PermissionSetArgs),
    #[command(about = "Print the permission request a conversation run is waiting on")]
    Pending(PermissionPendingArgs),
    #[command(about = "Answer the permission request a conversation run is waiting on")]
    Respond(PermissionRespondArgs),
}

#[derive(Debug, Args)]
pub(crate) struct PermissionWorkspaceArgs {
    #[arg(
        long,
        help = "Workspace directory whose policy applies; defaults to the current directory"
    )]
    pub(crate) workspace: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(crate) struct PermissionShowArgs {
    #[command(flatten)]
    pub(crate) workspace: PermissionWorkspaceArgs,
}

#[derive(Debug, Args)]
pub(crate) struct PermissionPendingArgs {
    #[arg(long, help = "Conversation whose run is waiting on a permission")]
    pub(crate) conversation_id: String,
    #[arg(long, help = "Run that is waiting on a permission")]
    pub(crate) run_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct PermissionSetArgs {
    #[command(flatten)]
    pub(crate) workspace: PermissionWorkspaceArgs,
    #[arg(
        long,
        value_enum,
        help = "Permission posture for every chat in the workspace"
    )]
    pub(crate) mode: PermissionModeArgument,
    #[arg(
        long = "allow-tool",
        help = "Approve one exact, provider-neutral tool name without widening a category"
    )]
    pub(crate) allowed_tools: Vec<String>,
    #[arg(
        long = "allow-category",
        value_enum,
        help = "Approve a complete typed category, such as read or network"
    )]
    pub(crate) allowed_categories: Vec<PermissionCategoryArgument>,
    #[arg(long, help = "Confirm the one-time change into the broad bypass mode")]
    pub(crate) consent_bypass: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PermissionRespondArgs {
    #[arg(long, help = "Conversation whose run is waiting on a permission")]
    pub(crate) conversation_id: String,
    #[arg(long, help = "Run that is waiting on a permission")]
    pub(crate) run_id: String,
    #[arg(
        long,
        help = "Decision id of the pending request, from `gent permissions pending`"
    )]
    pub(crate) decision_id: String,
    #[arg(long, value_enum, help = "Answer to send to the provider")]
    pub(crate) decision: PermissionDecisionArgument,
    #[arg(long, help = "Receipt id to reuse when retrying the same answer")]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PermissionModeArgument {
    AskEveryTime,
    AutoAcceptEdits,
    Autonomous,
    Bypass,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PermissionDecisionArgument {
    Deny,
    ApproveOnce,
    ApproveExactTool,
    ApproveCategory,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PermissionCategoryArgument {
    Read,
    Edit,
    Command,
    Network,
    Provider,
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: PermissionCommand,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    match command {
        PermissionCommand::Show(args) => {
            let workspace_id =
                workspace::resolve(data_dir.clone(), no_autostart, args.workspace.workspace)
                    .await?;
            Ok(serde_json::to_value(
                current_for(data_dir, no_autostart, workspace_id).await?,
            )?)
        }
        PermissionCommand::Set(args) => Ok(serde_json::to_value(
            save(data_dir, no_autostart, args).await?,
        )?),
        PermissionCommand::Pending(args) => Ok(serde_json::to_value(
            agent_chat::pending(data_dir, no_autostart, args.conversation_id, args.run_id).await?,
        )?),
        PermissionCommand::Respond(args) => {
            agent_chat::respond_decision(
                data_dir,
                no_autostart,
                args.conversation_id,
                args.run_id,
                args.decision_id,
                args.decision.into(),
                args.receipt_id,
            )
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
        return Err(CliError::new(
            Failure::ConsentRequired,
            "changing to bypass mode requires --consent-bypass",
        )
        .into());
    }
    let workspace_id =
        workspace::resolve(data_dir.clone(), no_autostart, args.workspace.workspace).await?;
    let current = current_for(data_dir.clone(), no_autostart, workspace_id.clone()).await?;
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
        &workspace_id,
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
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let response = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return a permission policy frame")?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| {
            "daemon returned a permission policy response with a different request".into()
        })
}

#[cfg(test)]
#[path = "permissions_cli_tests.rs"]
mod tests;
