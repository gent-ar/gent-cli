use std::path::PathBuf;

use clap::{Args, ValueEnum};
use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRunId, AgentChatSelection, ContextPolicy};

use gent_protocol::model_catalog::ModelCatalog;

#[derive(Debug, Args)]
pub(crate) struct SwitchArgs {
    #[arg(long, help = "Conversation whose selection changes")]
    pub(crate) conversation_id: String,
    #[arg(
        long,
        help = "Run to branch from [default: the conversation's current run]"
    )]
    pub(crate) parent_run_id: Option<String>,
    #[command(flatten)]
    pub(crate) selection: super::SelectionArgs,
    #[arg(
        long,
        value_enum,
        default_value_t = Context::Preserve,
        help = "Keep the conversation history for the new run, or start it with a clear context"
    )]
    pub(crate) context: Context,
    #[arg(long, help = "Client request id used to correlate the reply")]
    pub(crate) request_id: Option<String>,
    #[arg(long, help = "Receipt id; reuse it to retry the same switch safely")]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Context {
    Preserve,
    Clear,
}

pub(crate) fn frame(args: SwitchArgs) -> Result<AgentChatIntentFrame, String> {
    inheriting_frame(args, None, None)
}

fn inheriting_frame(
    args: SwitchArgs,
    parent: Option<&AgentChatSelection>,
    catalog: Option<&ModelCatalog>,
) -> Result<AgentChatIntentFrame, String> {
    let parent_run_id = args
        .parent_run_id
        .ok_or("a switch needs a durable current run")?;
    let request = args.selection.request();
    if parent.is_none()
        && (request.provider.is_none()
            || request.model.is_none()
            || request.effort.is_none()
            || request.mode.is_none())
    {
        return Err("a switch needs an explicit or inherited selection".into());
    }
    let selection = request.resolve(parent, catalog)?;
    Ok(selection_frame(
        args.conversation_id,
        parent_run_id,
        selection,
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
    let request = &args.selection;
    if args.parent_run_id.is_some()
        && request.provider.is_some()
        && request.model.is_some()
        && request.effort.is_some()
        && request.mode.is_some()
    {
        return frame(args).map_err(Into::into);
    }
    let detail =
        super::reads::detail(data_dir.clone(), no_autostart, args.conversation_id.clone()).await?;
    let parent_run_id = args
        .parent_run_id
        .get_or_insert_with(|| detail.current_run_id.clone())
        .clone();
    let Some(parent) = detail.runs.iter().find(|run| run.run_id == parent_run_id) else {
        return Err("daemon returned an invalid current run for this conversation".into());
    };
    let catalog = super::selection_catalog(
        data_dir,
        no_autostart,
        &args.selection.clone().request(),
        Some(&parent.selection),
    )
    .await?;
    inheriting_frame(args, Some(&parent.selection), catalog.as_ref()).map_err(Into::into)
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
