use std::path::PathBuf;

use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRequestId, AgentChatSelection, ReceiptId};

use super::{ChatCommand, PromptArgs, effort, interrupt, mode, model, provider, resume, switch};

pub(crate) fn frame(
    action: ChatCommand,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    Ok(match action {
        ChatCommand::Create(args) => AgentChatIntentFrame::CreateConversation {
            request_id: request_id(args.request_id),
            receipt_id: receipt_id(args.receipt_id),
            workspace_path: workspace_path(args.workspace)?,
            selection: AgentChatSelection {
                provider: provider(args.provider),
                model: model(args.provider, args.model),
                effort: effort(args.effort),
                mode: mode(args.mode),
            },
        },
        ChatCommand::Send(args) => prompt_frame(args, false, Vec::new()),
        ChatCommand::Resume(args) => resume::frame(args, Vec::new()),
        ChatCommand::Queue(args) => prompt_frame(args, true, Vec::new()),
        ChatCommand::Interrupt(args) => interrupt::frame(args),
        ChatCommand::Switch(args) | ChatCommand::Fork(args) => switch::frame(args)?,
        ChatCommand::Follow(_) => unreachable!("long-lived subscriptions bypass one-shot frames"),
        ChatCommand::FollowTurn(_) => unreachable!("turn follow bypasses one-shot frames"),
        ChatCommand::Summary(_) | ChatCommand::Detail(_) | ChatCommand::Transcript(_) => {
            unreachable!("agent-chat reads bypass intent frames")
        }
    })
}

pub(crate) fn workspace_path(value: Option<PathBuf>) -> Result<String, Box<dyn std::error::Error>> {
    value
        .unwrap_or(std::env::current_dir()?)
        .into_os_string()
        .into_string()
        .map_err(|_| "workspace path must be valid UTF-8".into())
}

pub(crate) fn prompt_frame(
    args: PromptArgs,
    queued: bool,
    attachment_ids: Vec<String>,
) -> AgentChatIntentFrame {
    let value = (
        request_id(args.request_id),
        receipt_id(args.receipt_id),
        AgentChatConversationId(args.conversation_id),
        args.text,
    );
    if queued && args.tool_sources.is_empty() {
        AgentChatIntentFrame::QueuePrompt {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
            attachment_ids,
        }
    } else if !queued && args.tool_sources.is_empty() {
        AgentChatIntentFrame::SendPrompt {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
            attachment_ids,
        }
    } else if queued {
        AgentChatIntentFrame::QueuePromptWithTools {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
            attachment_ids,
            tool_source_ids: args.tool_sources,
        }
    } else {
        AgentChatIntentFrame::SendPromptWithTools {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
            attachment_ids,
            tool_source_ids: args.tool_sources,
        }
    }
}

pub(crate) fn valid_reply(request: &AgentChatIntentFrame, response: &AgentChatIntentFrame) -> bool {
    if let Some(valid) = switch::valid_reply(request, response) {
        return valid;
    }
    match (request, response) {
        (
            AgentChatIntentFrame::CreateConversation {
                request_id,
                receipt_id,
                ..
            },
            AgentChatIntentFrame::Created {
                request_id: reply,
                receipt,
                conversation_id,
                run_id,
            },
        ) => {
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && !conversation_id.0.is_empty()
                && !run_id.0.is_empty()
        }
        (
            AgentChatIntentFrame::SendPrompt {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                ..
            }
            | AgentChatIntentFrame::QueuePrompt {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                ..
            }
            | AgentChatIntentFrame::SendPromptWithTools {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                ..
            }
            | AgentChatIntentFrame::QueuePromptWithTools {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                ..
            },
            AgentChatIntentFrame::Accepted {
                request_id: reply,
                receipt,
                conversation_id,
                run_id,
                turn_id,
                ..
            },
        ) => {
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && conversation_id == expected_conversation_id
                && !conversation_id.0.is_empty()
                && !run_id.0.is_empty()
                && !turn_id.is_empty()
        }
        (
            AgentChatIntentFrame::Interrupt {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                run_id: expected_run_id,
            },
            AgentChatIntentFrame::Interrupted {
                request_id: reply,
                receipt,
                conversation_id,
                run_id,
            },
        ) => {
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && conversation_id == expected_conversation_id
                && run_id == expected_run_id
                && !conversation_id.0.is_empty()
                && !run_id.0.is_empty()
        }
        _ => false,
    }
}

pub(crate) fn request_id(value: Option<String>) -> AgentChatRequestId {
    AgentChatRequestId(value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()))
}

pub(crate) fn receipt_id(value: Option<String>) -> ReceiptId {
    value.map_or_else(ReceiptId::new, ReceiptId)
}
