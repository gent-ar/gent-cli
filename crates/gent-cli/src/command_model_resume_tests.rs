use clap::Parser;

use super::{Args, CommandLine};
use crate::chat_cli::ChatCommand;

#[test]
fn chat_resume_uses_a_gent_conversation_identity_and_positional_prompt() {
    let args = Args::try_parse_from([
        "gent",
        "chat",
        "resume",
        "conversation-1",
        "continue from the selected run",
        "--request-id",
        "request-1",
        "--receipt-id",
        "receipt-1",
    ])
    .unwrap();
    let Some(CommandLine::Chat {
        action: ChatCommand::Resume(value),
    }) = args.command
    else {
        panic!("expected chat resume");
    };
    assert_eq!(value.conversation_id, "conversation-1");
    assert_eq!(value.text, "continue from the selected run");
    assert_eq!(value.request_id.as_deref(), Some("request-1"));
    assert_eq!(value.receipt_id.as_deref(), Some("receipt-1"));
}
