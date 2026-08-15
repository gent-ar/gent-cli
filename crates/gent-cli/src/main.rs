use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gent_protocol::{DependencyAction, DependencyProvider};

mod command_execution;
mod conversation_status;
mod conversation_timeline;
mod decision;
mod event_stream;
mod local_ipc;

use crate::decision::DecisionCommandLine;

#[derive(Debug, Parser)]
#[command(name = "gent", about = "Protocol-only client for a local gentd")]
struct Args {
    #[arg(long, env = "GENT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Fail if the local daemon is unavailable instead of starting one.
    #[arg(long, global = true)]
    no_autostart: bool,
    #[command(subcommand)]
    command: CommandLine,
}

#[derive(Debug, Subcommand)]
enum CommandLine {
    /// Read-only dependency discovery through the local daemon.
    Doctor,
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
    /// Read durable conversation, run, and active-turn status.
    Conversation {
        #[command(subcommand)]
        action: ConversationCommand,
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
enum DependencyCommand {
    /// Show a read-only install or update plan.
    Plan {
        action: DependencyAction,
        provider: DependencyProvider,
    },
    /// Confirm an install plan. No installer is started until this capability is configured.
    Install {
        provider: DependencyProvider,
        #[arg(long)]
        consent: bool,
    },
    /// Confirm an update plan. No updater is started until this capability is configured.
    Update {
        provider: DependencyProvider,
        #[arg(long)]
        consent: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConversationCommand {
    Status {
        #[arg(long)]
        conversation_id: String,
    },
    /// Read durable run/turn lineage and artifact provenance without transcript content.
    Timeline {
        #[arg(long)]
        conversation_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    command_execution::execute(Args::parse()).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use gent_protocol::{DependencyAction, DependencyProvider, WireFrame};

    use super::{Args, CommandLine, ConversationCommand, DependencyCommand};
    use crate::command_execution::dependency_frame;

    #[test]
    fn dependency_plan_is_read_only() {
        assert!(matches!(
            dependency_frame(&DependencyCommand::Plan {
                action: DependencyAction::Install,
                provider: DependencyProvider::Claude,
            }),
            WireFrame::DependencyPlanRequest(_)
        ));
    }

    #[test]
    fn dependency_install_requires_explicit_consent_flag() {
        assert!(matches!(
            dependency_frame(&DependencyCommand::Install {
                provider: DependencyProvider::Codex,
                consent: false,
            }),
            WireFrame::DependencyActionRequest(request) if !request.consent_granted
        ));
    }

    #[test]
    fn conversation_status_is_a_dedicated_read_only_command() {
        let args = Args::try_parse_from([
            "gent",
            "conversation",
            "status",
            "--conversation-id",
            "conversation-1",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            CommandLine::Conversation {
                action: ConversationCommand::Status { conversation_id }
            } if conversation_id == "conversation-1"
        ));
    }

    #[test]
    fn conversation_timeline_is_a_dedicated_read_only_command() {
        let args = Args::try_parse_from([
            "gent",
            "conversation",
            "timeline",
            "--conversation-id",
            "conversation-1",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            CommandLine::Conversation {
                action: ConversationCommand::Timeline { conversation_id }
            } if conversation_id == "conversation-1"
        ));
    }
}
