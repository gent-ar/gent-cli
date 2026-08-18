//! Command parsing model owned by the terminal boundary.

use clap::{Parser, Subcommand};
use gent_protocol::{DependencyAction, DependencyProvider};
use std::path::PathBuf;

use crate::{
    chat_cli, decision::DecisionCommandLine, goal_cli, permissions_cli, provider_auth_cli,
    reviewed_plan_cli, update_check::UpdateCommand,
};
#[derive(Debug, Parser)]
#[command(name = "gent", about = "Protocol-only client for a local gentd")]
#[command(version)]
pub(crate) struct Args {
    #[arg(long, env = "GENT_DATA_DIR")]
    pub(crate) data_dir: Option<PathBuf>,
    /// Fail if the local daemon is unavailable instead of starting one.
    #[arg(long, global = true)]
    pub(crate) no_autostart: bool,
    /// Open the read-only conversation browser.
    #[arg(long, global = true)]
    pub(crate) conversations: bool,
    /// Start or continue a local agent chat without entering the terminal browser.
    #[command(flatten)]
    pub(crate) direct_prompt: chat_cli::DirectPromptArgs,
    #[command(subcommand)]
    pub(crate) command: Option<CommandLine>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum CommandLine {
    /// Read-only dependency discovery through the local daemon.
    Doctor,
    /// Read the closed three-provider onboarding model without starting any provider.
    Onboarding,
    /// Review or explicitly consent to a public provider dependency action.
    Deps {
        #[command(subcommand)]
        action: DependencyCommand,
    },
    /// Submit or terminally settle a durable provider-neutral decision.
    Decision {
        #[command(subcommand)]
        action: DecisionCommandLine,
    },
    /// Inspect signed runtime-release availability or explicitly hand off a paired update.
    Update {
        #[command(subcommand)]
        action: UpdateCommand,
    },
    /// Read durable conversation, run, and active-turn status.
    Conversation {
        #[command(subcommand)]
        action: ConversationCommand,
    },
    /// Create a conversation or persist a user prompt through local agent-chat IPC.
    Chat {
        #[command(subcommand)]
        action: chat_cli::ChatCommand,
    },
    /// Read, approve, or reject a reviewed plan through the same Gent-owned IPC as the native app.
    Plan {
        #[command(subcommand)]
        action: reviewed_plan_cli::ReviewedPlanCommand,
    },
    /// Create, read, list, or settle durable provider-neutral conversation goals.
    Goal {
        #[command(subcommand)]
        action: goal_cli::GoalCommand,
    },
    /// Read or explicitly revise durable local permission preferences.
    Permissions {
        #[command(subcommand)]
        action: permissions_cli::PermissionCommand,
    },
    /// Inspect or request a secret-free Claude or Codex login choice through gentd.
    Auth {
        #[command(subcommand)]
        action: provider_auth_cli::ProviderAuthCommand,
    },
    Status,
    Submit {
        #[arg(long)]
        kind: String,
        #[arg(long, default_value = "{}")]
        payload: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Events {
        #[arg(long, default_value_t = 0)]
        after_cursor: u64,
        /// Keep the local IPC connection open and print cursor-ordered live batches.
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum DependencyCommand {
    /// Show a read-only install or update plan.
    Plan {
        action: DependencyAction,
        provider: DependencyProvider,
    },
    /// Confirm and run a reviewed public-provider installer.
    Install {
        provider: DependencyProvider,
        #[arg(long)]
        consent: bool,
        /// Reuse this key to safely retry the exact action after an interrupted client session.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Confirm and run a reviewed public-provider updater.
    Update {
        provider: DependencyProvider,
        #[arg(long)]
        consent: bool,
        /// Reuse this key to safely retry the exact action after an interrupted client session.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConversationCommand {
    /// List durable conversation identities and run counts without exposing messages.
    List,
    Status {
        #[arg(long)]
        conversation_id: String,
    },
    /// Read durable run/turn lineage and artifact provenance without transcript content.
    Timeline {
        #[arg(long)]
        conversation_id: String,
    },
    /// Read one future authority-gated activity snapshot or ordered delta.
    Activity {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        run_id: String,
        #[arg(long, default_value_t = 0)]
        after_cursor: u64,
    },
    /// Read a bounded page of locally stored user prompts from protected IPC.
    Content {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        before: Option<gent_types::ConversationContentCursor>,
        #[arg(long, default_value_t = 50)]
        limit: u16,
    },
}

#[cfg(test)]
#[path = "command_model_tests.rs"]
mod tests;
