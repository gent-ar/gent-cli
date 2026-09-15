use std::path::PathBuf;

use gent_drivers::{codex_prompt_runner::CodexPromptStart, codex_session::CodexTurnOptions};
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatMode, AgentChatPromptSaved, FrozenConversationContext};

use super::CodexPromptExecution;
use crate::public_driver_runtime::PublicDriversRuntime;

pub(super) struct LaunchSetup {
    working_directory: String,
    workspace_root: PathBuf,
    mode: AgentChatMode,
    selected_mcp_source_names: Vec<String>,
    turn_options: CodexTurnOptions,
}

impl LaunchSetup {
    pub(super) fn read<L, D, R>(
        runtime: &PublicDriversRuntime<L, D, R>,
        prompt: &AgentChatPromptSaved,
    ) -> Result<Self, RuntimeError>
    where
        L: Clone
            + Ledger
            + gent_ports::RunLifecycleFactLedger
            + ConversationActivityLedger
            + TranscriptLedger
            + AgentChatPromptDispatchLedger
            + gent_ports::AgentChatReadLedger
            + gent_ports::AgentChatRunContextReader
            + gent_ports::ConversationContentReader
            + gent_ports::AgentChatWorkspaceLedger
            + gent_ports::PolicyLedger
            + gent_ports::ToolSourceLedger
            + gent_ports::AttachmentLedger
            + gent_ports::AgentChatConversationConfigLedger,
        D: CodexPromptExecution + Clone,
        R: PublicProviderResolver,
    {
        let run_id = prompt.run_id.0.clone();
        let workspace = runtime.workspace_for_run(&prompt.message.conversation_id, &run_id)?;
        let working_directory = workspace.canonical_path;
        let workspace_root = std::path::PathBuf::from(&working_directory);
        let selection = runtime.selection_for_run(&prompt.message.conversation_id, &run_id)?;
        let selected_sources = runtime.validate_tool_sources_for_run(
            &prompt.message.conversation_id,
            &run_id,
            &prompt.tool_source_ids,
        )?;
        let mut selected_mcp_source_names = selected_sources
            .iter()
            .map(|source| source.source_name.clone())
            .collect::<Vec<_>>();
        if !selected_mcp_source_names.is_empty() {
            selected_mcp_source_names
                .extend(crate::standalone_mcp_config::INTERNAL_SERVER_NAMES.map(str::to_owned));
        }
        let permission =
            crate::permission_workspace::policy_for(&runtime.ledger(), &workspace.workspace_id)?;
        let conversation_config = runtime
            .ledger()
            .current_conversation_config(&prompt.message.conversation_id)
            .map_err(|error| gent_ports::PublicProviderRunError::Failed(error.to_string()))?;
        let turn_options =
            gent_drivers::codex_session::CodexTurnOptions::from_selection_with_permissions(
                &selection,
                Some(&working_directory),
                permission.mode,
            )
            .map_err(|error| gent_ports::PublicProviderRunError::Failed(error.to_string()))?
            .with_conversation_config(
                conversation_config
                    .as_ref()
                    .and_then(|config| config.system_prompt.clone()),
                conversation_config
                    .as_ref()
                    .is_some_and(|config| config.append_system_prompt),
            );
        Ok(Self {
            working_directory,
            workspace_root,
            mode: selection.mode,
            selected_mcp_source_names,
            turn_options,
        })
    }

    pub(super) fn start<L, D, R>(
        self,
        runtime: &PublicDriversRuntime<L, D, R>,
        prompt: &AgentChatPromptSaved,
        fresh_context: Option<FrozenConversationContext>,
    ) -> Result<CodexPromptStart, RuntimeError>
    where
        L: Clone
            + Ledger
            + gent_ports::RunLifecycleFactLedger
            + ConversationActivityLedger
            + TranscriptLedger
            + AgentChatPromptDispatchLedger
            + gent_ports::AgentChatReadLedger
            + gent_ports::AgentChatRunContextReader
            + gent_ports::ConversationContentReader
            + gent_ports::AgentChatWorkspaceLedger
            + gent_ports::PolicyLedger
            + gent_ports::ToolSourceLedger
            + gent_ports::AttachmentLedger
            + gent_ports::AgentChatConversationConfigLedger,
        D: CodexPromptExecution + Clone,
        R: PublicProviderResolver,
    {
        let interrupted_reply = if fresh_context.is_none() {
            runtime.interrupted_reply_before(&prompt.message)?
        } else {
            None
        };
        let goal = runtime.active_goal_for(&prompt.message.conversation_id)?;
        let (prompt_text, attachments) =
            provider_input(runtime, &prompt.message, &prompt.run_id.0)?;
        Ok(CodexPromptStart {
            working_directory: Some(self.working_directory),
            workspace_root: self.workspace_root,
            workspace_access: gent_types::SandboxWorkspaceAccess::from_mode(self.mode),
            prompt: prompt_text,
            goal,
            fresh_context,
            turn_options: self.turn_options,
            attachments,
            selected_mcp_source_names: self.selected_mcp_source_names,
            interrupted_reply,
        })
    }
}

pub(super) fn provider_input<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    message: &gent_types::ConversationMessage,
    run_id: &str,
) -> Result<(String, Vec<serde_json::Value>), RuntimeError>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AttachmentLedger,
    D: CodexPromptExecution + Clone,
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
    if attachment_metadata.is_empty() {
        return Ok((message.text.clone(), Vec::new()));
    }
    let (attachment_root, codex_attachment_root) = runtime.attachment_roots()?;
    let resolved = crate::provider_attachments::resolve(
        &runtime.ledger(),
        &gent_store::FileAttachmentBlobs::open(attachment_root).map_err(|_| {
            gent_ports::PublicProviderRunError::Failed(
                "provider attachment storage is unavailable".into(),
            )
        })?,
        &message.turn_id,
    )
    .map_err(gent_ports::PublicProviderRunError::Failed)?;
    let images = crate::provider_attachments::codex_local_images(
        &resolved,
        &codex_attachment_root.join(run_id),
    )
    .map_err(gent_ports::PublicProviderRunError::Failed)?;
    Ok((
        crate::provider_attachments::prompt_with_files(&message.text, &resolved),
        images,
    ))
}
