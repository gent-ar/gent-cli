//! Thin `gent` composition root: parse then execute one typed local-protocol command.

mod auto_update_handoff;
mod chat_cli;
mod command_execution;
mod command_model;
mod conversation_activity;
mod conversation_content;
mod conversation_index;
mod conversation_status;
mod conversation_timeline;
mod decision;
mod event_stream;
mod local_ipc;
mod permissions_cli;
mod runtime_maintenance;
mod runtime_update_check;
mod terminal;
mod terminal_browser;
mod update_check;
mod update_handoff;

pub(crate) use command_model::{Args, CommandLine, ConversationCommand, DependencyCommand};
pub(crate) use update_check::UpdateCommand;

use clap::Parser;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    command_execution::execute(Args::parse()).await
}
