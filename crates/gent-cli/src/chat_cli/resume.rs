//! Gent-owned conversation resume command.

use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use std::path::PathBuf;

use super::PromptArgs;

#[derive(Debug, Args)]
pub(crate) struct ResumeArgs {
    #[arg(value_name = "CONVERSATION_ID", help = "Conversation to continue")]
    pub(crate) conversation_id: String,
    #[arg(value_name = "PROMPT", help = "Prompt text")]
    pub(crate) text: String,
    #[arg(long, help = "Client request id used to correlate the reply")]
    pub(crate) request_id: Option<String>,
    #[arg(long, help = "Receipt id; reuse it to retry the same prompt safely")]
    pub(crate) receipt_id: Option<String>,
    #[arg(
        long = "attach",
        value_name = "PATH",
        help = "Attach a local file to the prompt (repeatable)"
    )]
    pub(crate) attachments: Vec<PathBuf>,
    #[arg(
        long,
        help = "Print machine-readable JSON frames instead of the streamed reply"
    )]
    pub(crate) json: bool,
}

pub(crate) fn frame(args: ResumeArgs, attachment_ids: Vec<String>) -> AgentChatIntentFrame {
    super::prompt_frame(
        PromptArgs {
            conversation_id: args.conversation_id,
            text: args.text,
            request_id: args.request_id,
            receipt_id: args.receipt_id,
            attachments: Vec::new(),
            tool_sources: Vec::new(),
            json: false,
        },
        false,
        attachment_ids,
    )
}
