//! Typed client-side construction and validation for a durable selection switch.

use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRunId, AgentChatSelection};

use super::{Effort, Mode, Provider};

#[derive(Debug, Args)]
pub(crate) struct SwitchArgs {
    #[arg(long)]
    pub(crate) conversation_id: String,
    #[arg(long)]
    pub(crate) parent_run_id: String,
    #[arg(long, value_enum)]
    pub(crate) provider: Provider,
    #[arg(long)]
    pub(crate) model: String,
    #[arg(long, value_enum, default_value_t = Effort::Medium)]
    pub(crate) effort: Effort,
    #[arg(long, value_enum, default_value_t = Mode::Ask)]
    pub(crate) mode: Mode,
    #[arg(long)]
    pub(crate) request_id: Option<String>,
    #[arg(long)]
    pub(crate) receipt_id: Option<String>,
}

pub(crate) fn frame(args: SwitchArgs) -> AgentChatIntentFrame {
    AgentChatIntentFrame::SwitchSelection {
        request_id: super::request_id(args.request_id),
        receipt_id: super::receipt_id(args.receipt_id),
        conversation_id: AgentChatConversationId(args.conversation_id),
        parent_run_id: AgentChatRunId(args.parent_run_id),
        selection: AgentChatSelection {
            provider: super::provider(args.provider),
            model: args.model,
            effort: super::effort(args.effort),
            mode: super::mode(args.mode),
        },
    }
}

pub(crate) fn valid_reply(
    request: &AgentChatIntentFrame,
    response: &AgentChatIntentFrame,
) -> Option<bool> {
    let (
        AgentChatIntentFrame::SwitchSelection {
            request_id,
            receipt_id,
            conversation_id,
            parent_run_id,
            ..
        },
        AgentChatIntentFrame::Switched {
            request_id: reply,
            receipt,
            conversation_id: reply_conversation,
            parent_run_id: reply_parent,
            run_id,
            ..
        },
    ) = (request, response)
    else {
        return None;
    };
    Some(
        reply == request_id
            && receipt.receipt_id == *receipt_id
            && reply_conversation == conversation_id
            && reply_parent == parent_run_id
            && !run_id.0.is_empty(),
    )
}
