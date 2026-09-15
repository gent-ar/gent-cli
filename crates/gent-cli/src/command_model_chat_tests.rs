use clap::Parser;

use super::{Args, CommandLine};
use crate::chat_cli::ChatCommand;

#[test]
fn selection_switch_parses_clear_context_without_losing_the_selected_model() {
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "switch",
        "--conversation-id",
        "conversation-1",
        "--parent-run-id",
        "run-1",
        "--provider",
        "gent",
        "--model",
        "claurst-main",
        "--effort",
        "low",
        "--mode",
        "agent",
        "--context",
        "clear",
    ])
    .unwrap();
    let Some(CommandLine::Chat {
        action: ChatCommand::Switch(value),
    }) = args.command
    else {
        panic!("expected selection switch");
    };
    assert!(matches!(
        value.selection.provider,
        Some(crate::chat_cli::Provider::Gent)
    ));
    assert_eq!(value.selection.model.as_deref(), Some("claurst-main"));
    assert!(matches!(
        value.selection.effort,
        Some(gent_types::AgentChatEffort::Low)
    ));
    assert!(matches!(
        value.selection.mode,
        Some(crate::chat_cli::Mode::Agent)
    ));
    assert!(matches!(
        value.context,
        crate::chat_cli::switch::Context::Clear
    ));
}

#[test]
fn fork_parses_a_context_preserving_provider_change() {
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "fork",
        "--conversation-id",
        "conversation-1",
        "--parent-run-id",
        "run-1",
        "--provider",
        "codex",
        "--model",
        "gpt-5.6",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Chat {
            action: ChatCommand::Fork(_)
        })
    ));
}

#[test]
fn create_without_a_choice_leaves_the_default_to_gentd() {
    let args = Args::try_parse_from(["gent", "chat", "create"]).unwrap();
    let Some(CommandLine::Chat {
        action: ChatCommand::Create(value),
    }) = args.command
    else {
        panic!("expected conversation creation");
    };
    assert!(value.selection.request().is_empty());
    assert!(Args::try_parse_from(["gent", "chat", "create", "--provider", "codex"]).is_ok());
    assert!(Args::try_parse_from(["gent", "chat", "create", "--mode", "ask"]).is_ok());
}

#[test]
fn interrupt_defaults_to_the_current_run_of_the_conversation() {
    assert!(
        Args::try_parse_from([
            "gent",
            "chat",
            "interrupt",
            "--conversation-id",
            "conversation-1",
        ])
        .is_ok()
    );
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "interrupt",
        "--conversation-id",
        "conversation-1",
        "--run-id",
        "run-1",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Chat {
            action: ChatCommand::Interrupt(_)
        })
    ));
}
