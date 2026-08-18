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
