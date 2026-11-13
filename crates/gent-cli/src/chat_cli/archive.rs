use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::AgentChatConversationId;

#[derive(Debug, Args)]
pub(crate) struct ArchiveArgs {
    #[arg(long = "conversation-id", help = "Conversation to archive")]
    pub(crate) conversation: String,
    #[arg(
        long = "request-id",
        help = "Client request id used to correlate the reply"
    )]
    pub(crate) request: Option<String>,
    #[arg(
        long = "receipt-id",
        help = "Receipt id; reuse it to retry the same archive safely"
    )]
    pub(crate) receipt: Option<String>,
}

pub(crate) fn frame(args: ArchiveArgs) -> AgentChatIntentFrame {
    AgentChatIntentFrame::ArchiveConversation {
        request_id: super::request_id(args.request),
        receipt_id: super::receipt_id(args.receipt),
        conversation_id: AgentChatConversationId(args.conversation),
    }
}
