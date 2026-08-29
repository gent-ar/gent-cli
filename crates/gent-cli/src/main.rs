//! Thin `gent` composition root: parse then execute one typed local-protocol command.

mod auto_update_handoff;
mod automation_cli;
mod chat_cli;
mod chat_command;
mod command_execution;
mod command_model;
mod conversation_activity;
mod conversation_content;
mod conversation_index;
mod conversation_status;
mod conversation_timeline;
mod decision;
mod direct_prompt;
mod direct_prompt_execution;
mod event_stream;
mod forge_cli;
mod goal_cli;
mod local_ipc;
mod local_models_cli;
mod mcp_server;
mod orchestration_cli;
mod permissions_cli;
mod prompt_templates_cli;
mod provider_auth_cli;
mod provider_lifecycle_cli;
mod reviewed_plan_cli;
mod runtime_activation;
mod runtime_maintenance;
mod runtime_update_check;
mod session_cli;
mod side_question_cli;
mod terminal;
mod terminal_browser;
mod terminal_preferences;
mod update_check;
mod update_command;
mod update_handoff;
mod workspace_documents_cli;
mod workspace_git_cli;

pub(crate) use command_model::{
    Args, AutomationCommand, CommandLine, ConversationCommand, DependencyCommand, ForgeCommand,
    SessionCommand,
};
pub(crate) use runtime_activation::RuntimeCommand;

use clap::Parser;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    command_execution::execute(Args::parse()).await
}
