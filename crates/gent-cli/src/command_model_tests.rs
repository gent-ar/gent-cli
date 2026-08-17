use clap::Parser;
use gent_protocol::{DependencyAction, DependencyProvider, WireFrame};

use super::{Args, CommandLine, ConversationCommand, DependencyCommand};
use crate::command_execution::dependency_plan_frame;
use crate::{
    chat_cli::ChatCommand,
    update_check::{UpdateChannel, UpdateCommand},
};

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
        Some(CommandLine::Conversation {
            action: ConversationCommand::Status { conversation_id }
        }) if conversation_id == "conversation-1"
    ));
}

#[test]
fn conversation_list_is_a_dedicated_read_only_command() {
    let args = Args::try_parse_from(["gent", "conversation", "list"]).unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Conversation {
            action: ConversationCommand::List
        })
    ));
}

#[test]
fn default_and_conversations_flag_select_the_terminal_browser() {
    let default_args = Args::try_parse_from(["gent"]).unwrap();
    assert!(default_args.command.is_none());
    let browser_args = Args::try_parse_from(["gent", "--conversations"]).unwrap();
    assert!(browser_args.conversations);
    assert!(browser_args.command.is_none());
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
        Some(CommandLine::Conversation {
            action: ConversationCommand::Timeline { conversation_id }
        }) if conversation_id == "conversation-1"
    ));
}

#[test]
fn conversation_activity_parses_its_cursor_bound_identity() {
    let args = Args::try_parse_from([
        "gent",
        "conversation",
        "activity",
        "--conversation-id",
        "conversation-1",
        "--run-id",
        "run-1",
        "--after-cursor",
        "9",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Conversation {
            action: ConversationCommand::Activity { conversation_id, run_id, after_cursor }
        }) if conversation_id == "conversation-1" && run_id == "run-1" && after_cursor == 9
    ));
}

#[test]
fn chat_transcript_is_a_bounded_read_command() {
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "transcript",
        "--conversation-id",
        "conversation-1",
        "--after-cursor",
        "9",
        "--limit",
        "20",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Chat { action: ChatCommand::Transcript(value) })
            if value.conversation_id == "conversation-1" && value.after_cursor == Some(9) && value.limit == 20
    ));
}

#[test]
fn decision_acknowledgement_commands_are_not_public_client_actions() {
    assert!(Args::try_parse_from(["gent", "decision", "ack", "--decision-id", "d1"]).is_err());
    assert!(Args::try_parse_from(["gent", "decision", "settle", "--decision-id", "d1"]).is_err());
}

#[test]
fn update_apply_requires_version_digest_and_explicit_consent() {
    let args = Args::try_parse_from(["gent", "update", "check", "--channel", "canary"]).unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Update {
            action: UpdateCommand::Check {
                channel: UpdateChannel::Canary
            }
        })
    ));
    assert!(Args::try_parse_from(["gent", "update", "apply"]).is_err());
    let apply = Args::try_parse_from([
        "gent",
        "update",
        "apply",
        "--version",
        "v1.2.3",
        "--expected-sha256",
        &"a".repeat(64),
        "--consent",
    ])
    .unwrap();
    assert!(matches!(
        apply.command,
        Some(CommandLine::Update {
            action: UpdateCommand::Apply { consent: true, .. }
        })
    ));
}
