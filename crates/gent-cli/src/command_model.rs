use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::{
    chat_cli, decision::DecisionCommandLine, goal_cli, local_models_cli, orchestration_cli,
    permissions_cli, prompt_templates_cli, provider_auth_cli, provider_lifecycle_cli,
    reviewed_plan_cli, runtime_activation::RuntimeCommand, side_question_cli,
    update_check::UpdateCommand, workspace_documents_cli, workspace_git_cli,
};
#[derive(Debug, Parser)]
#[command(
    name = "gent",
    about = "Chat with Claude, Codex, or local Gent models through the local gentd daemon",
    long_about = "Chat with Claude, Codex, or local Gent models through the local gentd daemon.\n\nRun `gent` to open the terminal client, or `gent \"<prompt>\"` to stream one reply.",
    after_help = EXIT_CODES
)]
#[command(version)]
pub(crate) struct Args {
    #[arg(
        long,
        global = true,
        help = "Gent data directory [default: $GENT_DATA_DIR, else the platform default]"
    )]
    pub(crate) data_dir: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Fail if gentd is not running instead of starting it"
    )]
    pub(crate) no_autostart: bool,
    #[arg(
        long,
        help = "Open the terminal client to browse conversations without sending prompts"
    )]
    pub(crate) conversations: bool,
    #[command(flatten)]
    pub(crate) direct_prompt: chat_cli::DirectPromptArgs,
    #[command(subcommand)]
    pub(crate) command: Option<CommandLine>,
}

const EXIT_CODES: &str = "Exit codes:\n  0  success\n  1  the request was rejected or failed\n  2  invalid command-line usage\n  3  gentd is unavailable\n  4  the requested item was not found\n  5  the action needs explicit consent\n  6  the turn failed\n  7  a turn or model download was interrupted or cancelled";

#[derive(Debug, Subcommand)]
pub(crate) enum CommandLine {
    #[command(about = "Check which provider CLIs and runtimes gentd can find")]
    Doctor,
    #[command(about = "Show what each provider needs before it can be used")]
    Onboarding,
    #[command(about = "Review or consent to installing or updating a provider CLI")]
    Deps {
        #[command(subcommand)]
        action: DependencyCommand,
    },
    #[command(about = "Record or settle a low-level durable decision (developer tool)")]
    Decision {
        #[command(subcommand)]
        action: DecisionCommandLine,
    },
    #[command(about = "Check for, schedule, or apply Gent runtime updates")]
    Update {
        #[command(subcommand)]
        action: UpdateCommand,
    },
    #[command(about = "Install a Gent runtime release from a bootstrap directory")]
    Runtime {
        #[command(subcommand)]
        action: RuntimeCommand,
    },
    #[command(about = "List conversations and read their runs, turns, and activity")]
    Conversation {
        #[command(subcommand)]
        action: ConversationCommand,
    },
    #[command(about = "Create conversations, send prompts, and manage running turns")]
    Chat {
        #[command(subcommand)]
        action: chat_cli::ChatCommand,
    },
    #[command(about = "Review, approve, or reject a plan produced in plan mode")]
    Plan {
        #[command(subcommand)]
        action: reviewed_plan_cli::ReviewedPlanCommand,
    },
    #[command(about = "Set, pause, resume, clear, or show a conversation's goal")]
    Goal {
        #[command(subcommand)]
        action: goal_cli::GoalCommand,
    },
    #[command(about = "Submit or read a multi-agent task graph")]
    Orchestration {
        #[command(subcommand)]
        action: orchestration_cli::OrchestrationCommand,
    },
    #[command(about = "Show or change permission settings and answer permission requests")]
    Permissions {
        #[command(subcommand)]
        action: permissions_cli::PermissionCommand,
    },
    #[command(about = "Check or start Claude or Codex sign-in")]
    Auth {
        #[command(subcommand)]
        action: provider_auth_cli::ProviderAuthCommand,
    },
    #[command(about = "Manage Forge connectors for a workspace")]
    Forge {
        #[command(subcommand)]
        action: ForgeCommand,
    },
    #[command(about = "Create, list, and run workspace automations")]
    Automation {
        #[command(subcommand)]
        action: AutomationCommand,
    },
    #[command(about = "Create, select, and attach conversations to named sessions")]
    Sessions {
        #[command(subcommand)]
        action: SessionCommand,
    },
    #[command(about = "Serve every Gent MCP tool over stdio")]
    McpServer,
    #[command(about = "Serve Gent MCP tools over stdio, optionally for one domain")]
    Mcp {
        #[arg(
            value_parser = ["goal", "automations", "forge"],
            help = "Tool domain to serve [default: every domain]"
        )]
        domain: Option<String>,
    },
    #[command(about = "Check whether a held prompt can run, or consent to its provider install")]
    Provider {
        #[command(subcommand)]
        action: provider_lifecycle_cli::ProviderLifecycleCommand,
    },
    #[command(about = "List every provider's models and efforts, and download local Gent models")]
    Models {
        #[command(subcommand)]
        action: local_models_cli::LocalModelsCommand,
    },
    #[command(about = "Create, list, render, and delete prompt templates")]
    Templates {
        #[command(subcommand)]
        action: prompt_templates_cli::PromptTemplateCommand,
    },
    #[command(about = "List documents in a workspace")]
    Documents {
        #[command(subcommand)]
        action: workspace_documents_cli::WorkspaceDocumentsCommand,
    },
    #[command(about = "Read Git status and nested repositories of a workspace")]
    WorkspaceGit {
        #[command(subcommand)]
        action: workspace_git_cli::WorkspaceGitCommand,
    },
    #[command(about = "Ask, cancel, or list side questions about a conversation")]
    SideQuestion {
        #[command(subcommand)]
        action: side_question_cli::SideQuestionCommand,
    },
    #[command(about = "Print the resolved data directory without contacting gentd")]
    DataDir,
    #[command(about = "Show gentd's host status and negotiated capabilities")]
    Status,
    #[command(about = "Submit a raw durable command to gentd (developer tool)")]
    Submit {
        #[arg(long, help = "Command kind")]
        kind: String,
        #[arg(long, default_value = "{}", help = "Command payload as JSON")]
        payload: String,
        #[arg(
            long,
            help = "Idempotency key; reuse it to retry the same command safely"
        )]
        idempotency_key: Option<String>,
    },
    #[command(about = "Print gentd's durable event log")]
    Events {
        #[arg(
            long,
            default_value_t = 0,
            help = "Print only events after this cursor"
        )]
        after_cursor: u64,
        #[arg(
            long,
            help = "Keep the connection open and print new events as they happen"
        )]
        follow: bool,
    },
}

#[path = "command_model_domains.rs"]
mod domains;
pub(crate) use domains::{
    AutomationCommand, ConversationCommand, DependencyCommand, ForgeCommand, SessionCommand,
};

#[cfg(test)]
#[path = "command_model_local_models_tests.rs"]
mod local_models_tests;
#[cfg(test)]
#[path = "command_model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "command_model_chat_tests.rs"]
mod chat_tests;

#[cfg(test)]
#[path = "command_model_resume_tests.rs"]
mod resume_tests;

#[cfg(test)]
#[path = "command_model_help_tests.rs"]
mod help_tests;
