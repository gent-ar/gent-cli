use super::{Args, CommandLine, ConversationCommand};
use crate::{
    chat_cli::ChatCommand,
    update_check::{UpdateChannel, UpdateCommand},
};
use clap::Parser;

#[path = "command_model_dependency_tests.rs"]
mod dependency_tests;

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
fn positional_prompt_selects_the_typed_prompt_first_flow() {
    let args = Args::try_parse_from([
        "gent",
        "write a status report",
        "--provider",
        "codex",
        "--mode",
        "agent",
    ])
    .unwrap();
    assert_eq!(
        args.direct_prompt.prompt.as_deref(),
        Some("write a status report")
    );
    assert!(args.command.is_none());
    assert!(Args::try_parse_from(["gent", "--provider", "codex"]).is_err());
}

#[test]
fn positional_prompt_defaults_to_local_gent_agent_mode() {
    let args = Args::try_parse_from(["gent", "summarize this project"]).unwrap();
    assert!(matches!(
        args.direct_prompt.provider,
        crate::chat_cli::Provider::Claurst
    ));
    assert!(matches!(
        args.direct_prompt.mode,
        crate::chat_cli::Mode::Agent
    ));
}

#[test]
fn an_existing_conversation_rejects_ignored_selection_flags() {
    let args = Args::try_parse_from([
        "gent",
        "continue this",
        "--conversation-id",
        "conversation-1",
    ])
    .unwrap();
    assert_eq!(
        args.direct_prompt.conversation_id.as_deref(),
        Some("conversation-1")
    );
    for selection_flag in [
        ["--provider", "claude"].as_slice(),
        ["--model", "sonnet"].as_slice(),
        ["--effort", "high"].as_slice(),
        ["--mode", "plan"].as_slice(),
    ] {
        let mut command = vec![
            "gent",
            "continue this",
            "--conversation-id",
            "conversation-1",
        ];
        command.extend_from_slice(selection_flag);
        assert!(Args::try_parse_from(command).is_err());
    }
}

#[test]
fn selection_switch_parses_a_context_preserving_claude_plan() {
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "switch",
        "--conversation-id",
        "conversation-1",
        "--parent-run-id",
        "run-1",
        "--provider",
        "claude",
        "--model",
        "sonnet",
        "--effort",
        "high",
        "--mode",
        "plan",
        "--context",
        "preserve",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Chat {
            action: ChatCommand::Switch(_)
        })
    ));
}

#[test]
fn chat_subcommands_remain_distinct_from_a_positional_prompt() {
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "send",
        "--conversation-id",
        "conversation-1",
        "--text",
        "hello",
    ])
    .unwrap();
    assert!(args.direct_prompt.prompt.is_none());
    assert!(matches!(args.command, Some(CommandLine::Chat { .. })));
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

#[path = "command_model_decision_visibility_tests.rs"]
mod decision_visibility_tests;

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
