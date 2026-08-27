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
        "claurst",
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
    assert!(matches!(value.provider, crate::chat_cli::Provider::Claurst));
    assert_eq!(value.model, "claurst-main");
    assert!(matches!(value.effort, crate::chat_cli::Effort::Low));
    assert!(matches!(value.mode, crate::chat_cli::Mode::Agent));
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
fn create_uses_the_same_default_selection_as_prompt_first_chat() {
    let args = Args::try_parse_from(["gent", "chat", "create"]).unwrap();
    let Some(CommandLine::Chat {
        action: ChatCommand::Create(value),
    }) = args.command
    else {
        panic!("expected conversation creation");
    };
    assert!(matches!(value.provider, crate::chat_cli::Provider::Claurst));
    assert_eq!(value.model, gent_protocol::DEFAULT_LOCAL_MODEL_ID);
    assert!(matches!(value.mode, crate::chat_cli::Mode::Agent));
}

#[test]
fn interrupt_requires_the_conversation_and_run_identity() {
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
