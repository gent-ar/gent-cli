//! Typed client-side construction and validation for a durable selection switch.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRunId, AgentChatSelection, ContextPolicy};

use super::{Effort, Mode, Provider, model};

#[derive(Debug, Args)]
pub(crate) struct SwitchArgs {
    #[arg(long)]
    pub(crate) conversation_id: String,
    #[arg(long)]
    pub(crate) parent_run_id: Option<String>,
    #[arg(long, value_enum)]
    pub(crate) provider: Provider,
    #[arg(long)]
    pub(crate) model: String,
    #[arg(long, value_enum, default_value_t = Effort::Medium)]
    pub(crate) effort: Effort,
    #[arg(long, value_enum, default_value_t = Mode::Ask)]
    pub(crate) mode: Mode,
    /// Explicitly preserve durable context or begin this child with an empty context.
    #[arg(long, value_enum, default_value_t = Context::Preserve)]
    pub(crate) context: Context,
    #[arg(long)]
    pub(crate) request_id: Option<String>,
    #[arg(long)]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Context {
    Preserve,
    Clear,
}

pub(crate) fn frame(args: SwitchArgs) -> Result<AgentChatIntentFrame, &'static str> {
    let parent_run_id = args
        .parent_run_id
        .ok_or("a switch needs a durable current run")?;
    Ok(selection_frame(
        args.conversation_id,
        parent_run_id,
        AgentChatSelection {
            provider: super::provider(args.provider),
            model: model(args.provider, args.model),
            effort: super::effort(args.effort),
            mode: super::mode(args.mode),
        },
        match args.context {
            Context::Preserve => ContextPolicy::Preserve,
            Context::Clear => ContextPolicy::Clear,
        },
        args.request_id,
        args.receipt_id,
    ))
}

pub(crate) async fn resolve(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    mut args: SwitchArgs,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    if args.parent_run_id.is_none() {
        let detail =
            super::reads::detail(data_dir, no_autostart, args.conversation_id.clone()).await?;
        if detail.current_run_id.is_empty()
            || !detail
                .runs
                .iter()
                .any(|run| run.run_id == detail.current_run_id)
        {
            return Err("daemon returned an invalid current run for this conversation".into());
        }
        args.parent_run_id = Some(detail.current_run_id);
    }
    frame(args).map_err(Into::into)
}

/// Switches one known terminal parent through the same checked IPC path as `gent chat switch`.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    parent_run_id: String,
    selection: AgentChatSelection,
    context_policy: ContextPolicy,
) -> Result<AgentChatRunId, Box<dyn std::error::Error>> {
    let request = selection_frame(
        conversation_id,
        parent_run_id,
        selection,
        context_policy,
        None,
        None,
    );
    let response = super::exchange(data_dir, no_autostart, request.clone()).await?;
    let AgentChatIntentFrame::Switched { run_id, .. } = response else {
        return Err("daemon did not return a switched selection".into());
    };
    Ok(run_id)
}

fn selection_frame(
    conversation_id: String,
    parent_run_id: String,
    selection: AgentChatSelection,
    context_policy: ContextPolicy,
    request_id: Option<String>,
    receipt_id: Option<String>,
) -> AgentChatIntentFrame {
    AgentChatIntentFrame::SwitchSelection {
        request_id: super::request_id(request_id),
        receipt_id: super::receipt_id(receipt_id),
        conversation_id: AgentChatConversationId(conversation_id),
        parent_run_id: AgentChatRunId(parent_run_id),
        selection,
        context_policy,
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
            context_policy,
            ..
        },
        AgentChatIntentFrame::Switched {
            request_id: reply,
            receipt,
            conversation_id: reply_conversation,
            parent_run_id: reply_parent,
            run_id,
            context_policy: reply_policy,
            context_through_ordinal,
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
            && reply_policy == context_policy
            && !run_id.0.is_empty()
            && (*context_policy != ContextPolicy::Clear || *context_through_ordinal == 0),
    )
}
