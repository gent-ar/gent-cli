use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::AgentChatPromptSaved;

use super::{ClaudePromptExecution, ClaudePromptStart};
use crate::public_driver_runtime::PublicDriversRuntime;

pub(super) type LaunchKey = (ClaudePromptStart, gent_types::PolicyRecord);

pub(super) fn prompt_start<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    prompt: &AgentChatPromptSaved,
    fresh_context: Option<gent_types::FrozenConversationContext>,
) -> Result<LaunchKey, RuntimeError>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::PolicyLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    let (start, permission) = launch_key(
        runtime,
        &prompt.message.conversation_id,
        &prompt.run_id.0,
        &prompt.tool_source_ids,
    )?;
    let goal = runtime.active_goal_for(&prompt.message.conversation_id)?;
    let (prompt_text, content) = provider_input(runtime, &prompt.message)?;
    Ok((
        ClaudePromptStart {
            prompt: prompt_text,
            goal,
            fresh_context,
            content,
            ..start
        },
        permission,
    ))
}

pub(super) fn fresh_input(
    context: &gent_types::FrozenConversationContext,
    prompt: &str,
) -> Result<String, RuntimeError> {
    gent_drivers::conversation_context_input::render_fresh_conversation_input(
        context,
        prompt,
        gent_drivers::claude_runner::MAX_CLAUDE_FRAME_BYTES,
    )
    .map(|input| input.prompt().to_owned())
    .map_err(|_| {
        gent_ports::PublicProviderRunError::Failed("Claude frozen context is invalid".into()).into()
    })
}

pub(super) fn launch_key<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    conversation_id: &str,
    run_id: &str,
    tool_source_ids: &[String],
) -> Result<LaunchKey, RuntimeError>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::PolicyLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    let workspace = runtime.workspace_for_run(conversation_id, run_id)?;
    let selection = runtime.selection_for_run(conversation_id, run_id)?;
    let selected_sources =
        runtime.validate_tool_sources_for_run(conversation_id, run_id, tool_source_ids)?;
    let mut selected_mcp_source_names = selected_sources
        .iter()
        .map(|source| source.source_name.clone())
        .collect::<Vec<_>>();
    if !selected_mcp_source_names.is_empty() {
        selected_mcp_source_names
            .extend(crate::standalone_mcp_config::INTERNAL_SERVER_NAMES.map(str::to_owned));
    }
    let permission = runtime.conversation_policy(conversation_id, &workspace.workspace_id)?;
    runtime.runner().validate_permission_binding(
        gent_types::SandboxPermissionBinding::new(
            conversation_id.to_owned(),
            run_id.to_owned(),
            permission.clone(),
            runtime.ledger().host_ingress()?.epoch,
        )
        .map_err(|_| {
            gent_ports::PublicProviderRunError::Failed("invalid provider permission binding".into())
        })?,
    )?;
    let conversation_config = runtime
        .ledger()
        .current_conversation_config(conversation_id)
        .map_err(|error| gent_ports::PublicProviderRunError::Failed(error.to_string()))?;
    let turn_options =
        gent_drivers::claude_turn_options::ClaudeTurnOptions::from_selection_with_permissions(
            &selection,
            permission.mode,
        )
        .map_err(|error| gent_ports::PublicProviderRunError::Failed(error.to_string()))?
        .with_thinking_enabled(
            conversation_config
                .as_ref()
                .and_then(|config| config.thinking_enabled),
        )
        .with_conversation_config(
            conversation_config
                .as_ref()
                .and_then(|config| config.system_prompt.clone()),
            conversation_config
                .as_ref()
                .is_some_and(|config| config.append_system_prompt),
            conversation_config
                .as_ref()
                .and_then(|config| config.max_turns),
            conversation_config
                .as_ref()
                .map(|config| config.disallowed_tools.clone())
                .unwrap_or_default(),
        );
    Ok((
        ClaudePromptStart {
            conversation_id: conversation_id.to_owned(),
            workspace_root: workspace.canonical_path.into(),
            workspace_access: gent_types::SandboxWorkspaceAccess::from_mode(selection.mode),
            prompt: String::new(),
            turn_options,
            goal: None,
            fresh_context: None,
            content: Vec::new(),
            selected_mcp_source_names,
            disallowed_mcp_tools: conversation_config
                .map(|config| config.disallowed_tools)
                .unwrap_or_default(),
            recreate_session: false,
        },
        permission,
    ))
}

pub(super) fn provider_input<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    message: &gent_types::ConversationMessage,
) -> Result<(String, Vec<serde_json::Value>), RuntimeError>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AttachmentLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    let attachment_metadata = runtime
        .ledger()
        .turn_attachments(&message.turn_id)
        .map_err(|error| {
            gent_ports::PublicProviderRunError::Failed(format!(
                "turn attachments are unavailable: {error}"
            ))
        })?;
    let attachments = if attachment_metadata.is_empty() {
        Vec::new()
    } else {
        let (attachment_root, _) = runtime.attachment_roots()?;
        crate::provider_attachments::resolve(
            &runtime.ledger(),
            &gent_store::FileAttachmentBlobs::open(attachment_root).map_err(|_| {
                gent_ports::PublicProviderRunError::Failed(
                    "provider attachment storage is unavailable".into(),
                )
            })?,
            &message.turn_id,
        )
        .map_err(gent_ports::PublicProviderRunError::Failed)?
    };
    Ok((
        crate::provider_attachments::prompt_with_files(&message.text, &attachments),
        crate::provider_attachments::claude_content(&attachments),
    ))
}
