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
    use crate::command_execution::dependency_plan_frame;

    #[test]
    fn dependency_plan_is_read_only() {
        assert!(matches!(
            dependency_plan_frame(DependencyProvider::Claude, DependencyAction::Install),
            WireFrame::DependencyPlanRequest(_)
        ));
    }

    #[test]
    fn dependency_install_parses_a_retry_key() {
        let args = Args::try_parse_from([
            "gent",
            "deps",
            "install",
            "codex",
            "--consent",
            "--idempotency-key",
            "retry-1",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            CommandLine::Deps {
                action: DependencyCommand::Install { idempotency_key: Some(key), .. }
            } if key == "retry-1"
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

    #[test]
    fn decision_acknowledgement_commands_are_not_public_client_actions() {
        assert!(Args::try_parse_from(["gent", "decision", "ack", "--decision-id", "d1"]).is_err());
        assert!(
            Args::try_parse_from(["gent", "decision", "settle", "--decision-id", "d1"]).is_err()
        );
    }
}
