//! Gent-owned conversation resume command.

use clap::Args;
use gent_protocol::AgentChatIntentFrame;

use super::PromptArgs;

/// Resumes a durable Gent conversation rather than a provider-native session.
#[derive(Debug, Args)]
pub(crate) struct ResumeArgs {
    #[arg(value_name = "CONVERSATION_ID")]
    pub(crate) conversation_id: String,
    #[arg(value_name = "PROMPT")]
    pub(crate) text: String,
    #[arg(long)]
    pub(crate) request_id: Option<String>,
    #[arg(long)]
    pub(crate) receipt_id: Option<String>,
}

pub(crate) fn frame(args: ResumeArgs) -> AgentChatIntentFrame {
    super::prompt_frame(
        PromptArgs {
            conversation_id: args.conversation_id,
            text: args.text,
            request_id: args.request_id,
            receipt_id: args.receipt_id,
        },
        false,
    )
}
