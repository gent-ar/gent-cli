//! Thin `gent` composition root: parse then execute one typed local-protocol command.

mod auto_update_handoff;
mod automation_cli;
mod chat_cli;
mod chat_command;
mod cli_error;
mod command_catalog_cli;
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
mod model_catalog_cli;
mod orchestration_cli;
mod permissions_cli;
mod prompt_hold;
mod prompt_queue;
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
async fn main() -> std::process::ExitCode {
    match command_execution::execute(Args::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => cli_error::report(error.as_ref()),
    }
}
