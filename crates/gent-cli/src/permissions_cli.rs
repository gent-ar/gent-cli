use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::PermissionPolicyFrame;
use gent_types::{PermissionCategory, PermissionMode, PolicyRecord};
use std::path::PathBuf;

use crate::cli_error::{CliError, Failure};

pub(crate) mod agent_chat;
mod conversions;
mod mode;
mod policy_exchange;
mod workspace;
use conversions::command_grants;
pub(crate) use mode::set_mode;
use policy_exchange::{exchange, policy};

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
    #[arg(
        long,
        help = "Conversation whose running turn is waiting on a permission"
    )]
    pub(crate) conversation_id: String,
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
    #[arg(
        long = "allow-command",
        help = "Approve a tool's simple command by pattern, as TOOL=PATTERN (e.g. 'Bash=npm run *')"
    )]
    pub(crate) allowed_commands: Vec<String>,
    #[arg(long, help = "Confirm the one-time change into the broad bypass mode")]
    pub(crate) consent_bypass: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PermissionRespondArgs {
    #[arg(
        long,
        help = "Conversation whose running turn is waiting on a permission"
    )]
    pub(crate) conversation_id: String,
    #[arg(
        long,
        help = "Decision id of the pending request, from `gent permissions pending`"
    )]
    pub(crate) decision_id: String,
    #[arg(long, value_enum, help = "Answer to send to the provider")]
    pub(crate) decision: PermissionDecisionArgument,
    #[arg(long, help = "Receipt id to reuse when retrying the same answer")]
    pub(crate) receipt_id: Option<String>,
    #[arg(
        long,
        help = "Command pattern persisted by approve-command-pattern, e.g. 'git status'"
    )]
    pub(crate) command_pattern: Option<String>,
    #[arg(
        long,
        value_parser = json_object,
        help = "JSON object answering a question request, the input `/answer` sends"
    )]
    pub(crate) answer: Option<serde_json::Value>,
}

fn json_object(raw: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .filter(serde_json::Value::is_object)
        .ok_or_else(|| "--answer requires a JSON object".into())
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
    ApproveCommandPattern,
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
            agent_chat::pending(data_dir, no_autostart, args.conversation_id).await?,
        )?),
        PermissionCommand::Respond(args) => {
            agent_chat::respond_decision(data_dir, no_autostart, args).await
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
    let allowed_commands = command_grants(&args.allowed_commands)?;
    let policy = policy(
        &workspace_id,
        revision,
        mode,
        allowed_tools,
        allowed_categories,
        allowed_commands,
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

#[cfg(test)]
#[path = "permissions_cli_tests.rs"]
mod tests;
