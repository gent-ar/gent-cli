use clap::Parser;
use gent_protocol::{DependencyAction, DependencyProvider, WireFrame};

use super::super::{Args, CommandLine, DependencyCommand};
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
        Some(CommandLine::Deps {
            action: DependencyCommand::Install { idempotency_key: Some(key), .. }
        }) if key == "retry-1"
    ));
}
