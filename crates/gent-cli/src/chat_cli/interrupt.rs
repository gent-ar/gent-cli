use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRunId};

#[derive(Debug, Args)]
pub(crate) struct InterruptArgs {
    #[arg(long = "conversation-id")]
    pub(crate) conversation: String,
    #[arg(long = "run-id")]
    pub(crate) run: String,
    #[arg(long = "request-id")]
    pub(crate) request: Option<String>,
    #[arg(long = "receipt-id")]
    pub(crate) receipt: Option<String>,
}

pub(crate) fn frame(args: InterruptArgs) -> AgentChatIntentFrame {
    AgentChatIntentFrame::Interrupt {
        request_id: super::request_id(args.request),
        receipt_id: super::receipt_id(args.receipt),
        conversation_id: AgentChatConversationId(args.conversation),
        run_id: AgentChatRunId(args.run),
    }
}
