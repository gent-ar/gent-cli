use std::path::PathBuf;

use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRunId};

#[derive(Debug, Args)]
pub(crate) struct InterruptArgs {
    #[arg(long = "conversation-id", help = "Conversation whose work is stopped")]
    pub(crate) conversation: String,
    #[arg(
        long = "run-id",
        help = "Run to interrupt [default: the conversation's current run]"
    )]
    pub(crate) run: Option<String>,
    #[arg(
        long = "request-id",
        help = "Client request id used to correlate the reply"
    )]
    pub(crate) request: Option<String>,
    #[arg(
        long = "receipt-id",
        help = "Receipt id; reuse it to retry the same interrupt safely"
    )]
    pub(crate) receipt: Option<String>,
}

pub(crate) fn frame(args: InterruptArgs) -> Result<AgentChatIntentFrame, &'static str> {
    let run = args
        .run
        .ok_or("an interrupt needs the conversation's current run")?;
    Ok(AgentChatIntentFrame::Interrupt {
        request_id: super::request_id(args.request),
        receipt_id: super::receipt_id(args.receipt),
        conversation_id: AgentChatConversationId(args.conversation),
        run_id: AgentChatRunId(run),
    })
}

pub(crate) async fn resolve(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    mut args: InterruptArgs,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    if args.run.is_none() {
        let detail =
            super::reads::detail(data_dir, no_autostart, args.conversation.clone()).await?;
        args.run = Some(detail.current_run_id);
    }
    frame(args).map_err(Into::into)
}
