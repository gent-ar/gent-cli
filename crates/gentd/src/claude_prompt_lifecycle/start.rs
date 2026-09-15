//! Durable-before-spawn transition for one claimed Claude prompt.

use std::collections::BTreeMap;

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, TranscriptLedger,
};
use gent_protocol::{DependencyProvider, PublicRunOutcome, PublicRunStartRequest};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatPromptSaved, HostEpoch};

use super::{Binding, ClaudePromptDispatchOutcome, ClaudePromptExecution, ClaudePromptStart};
use crate::public_driver_runtime::PublicDriversRuntime;

pub(super) fn prompt<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    runner: &D,
    coordinator_id: &str,
    active: &mut BTreeMap<String, Binding>,
    prompt: AgentChatPromptSaved,
    host_epoch: HostEpoch,
) -> Result<ClaudePromptDispatchOutcome, RuntimeError>
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
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    let fresh_context = runtime.launch_context(&prompt.message)?;
    let start = prompt_start(runtime, &prompt, fresh_context)?;
    if runner.has_claude_session(&run_id) {
        return submit(
            runtime,
            runner,
            coordinator_id,
            active,
            prompt,
            host_epoch,
            start.prompt,
            start.goal,
            start.content,
        );
    }
    runner.prepare_claude_prompt(run_id.clone(), start)?;
    if let Err(error) = runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch) {
        runner.cancel_claude_prompt(&run_id);
        return Err(error);
    }
    match runtime
        .runs()
        .start_or_resume(request(&run_id, coordinator_id, host_epoch))
    {
        Err(error) => {
            runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
            Err(error)
        }
        Ok(response) => match response.outcome {
            PublicRunOutcome::Started | PublicRunOutcome::Resumed => {
                if let Err(error) =
                    runtime.confirm_prompt_started(&message_id, coordinator_id, host_epoch)
                {
                    let _ = runner.interrupt(&run_id);
                    runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
                    return Err(error);
                }
                active.insert(
                    run_id.clone(),
                    Binding {
                        prompt,
                        sequence: 0,
                        settled: false,
                        interrupt_requested: false,
                        steers: Vec::new(),
                        session_recovery: super::SessionRecovery::NotNeeded,
                    },
                );
                Ok(ClaudePromptDispatchOutcome::Started { run_id })
            }
            PublicRunOutcome::Denied | PublicRunOutcome::LeaseContended => {
                runner.cancel_claude_prompt(&run_id);
                runtime.release_unstarted_prompt_launch(&message_id, coordinator_id, host_epoch)?;
                Ok(ClaudePromptDispatchOutcome::Empty)
            }
            PublicRunOutcome::Interrupted => {
                runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
                Ok(ClaudePromptDispatchOutcome::Unprovable { run_id })
            }
        },
    }
}

pub(super) fn prompt_start<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    prompt: &AgentChatPromptSaved,
    fresh_context: Option<gent_types::FrozenConversationContext>,
) -> Result<ClaudePromptStart, RuntimeError>
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
    let run_id = prompt.run_id.0.clone();
    let workspace = runtime.workspace_for_run(&prompt.message.conversation_id, &run_id)?;
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
        gent_drivers::claude_turn_options::ClaudeTurnOptions::from_selection_with_permissions(
            &selection,
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
            conversation_config
                .as_ref()
                .and_then(|config| config.max_turns),
            conversation_config
                .map(|config| config.disallowed_tools)
                .unwrap_or_default(),
        );
    let goal = runtime.active_goal_for(&prompt.message.conversation_id)?;
    let (prompt_text, content) = provider_input(runtime, &prompt.message)?;
    Ok(ClaudePromptStart {
        workspace_root: workspace.canonical_path.into(),
        workspace_access: gent_types::SandboxWorkspaceAccess::from_mode(selection.mode),
        prompt: prompt_text,
        turn_options,
        goal,
        fresh_context,
        content,
        selected_mcp_source_names,
        recreate_session: false,
    })
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

#[allow(clippy::too_many_arguments)]
fn submit<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    runner: &D,
    coordinator_id: &str,
    active: &mut BTreeMap<String, Binding>,
    prompt: AgentChatPromptSaved,
    host_epoch: HostEpoch,
    prompt_text: String,
    goal: Option<gent_types::GoalProjection>,
    content: Vec<serde_json::Value>,
) -> Result<ClaudePromptDispatchOutcome, RuntimeError>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AttachmentLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch)?;
    if let Err(error) = runner.submit_claude_prompt(&run_id, &prompt_text, goal.as_ref(), &content)
    {
        runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
        return Err(error.into());
    }
    if let Err(error) = runtime.confirm_prompt_started(&message_id, coordinator_id, host_epoch) {
        let _ = runner.interrupt(&run_id);
        runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
        return Err(error);
    }
    let binding = active.get_mut(&run_id).ok_or_else(super::missing_binding)?;
    binding.prompt = prompt;
    binding.settled = false;
    Ok(ClaudePromptDispatchOutcome::Started { run_id })
}

fn request(run_id: &str, coordinator_id: &str, host_epoch: HostEpoch) -> PublicRunStartRequest {
    PublicRunStartRequest {
        run_id: run_id.into(),
        coordinator_id: coordinator_id.into(),
        host_epoch,
        provider: DependencyProvider::Claude,
        executable: "daemon-resolved".into(),
        version: "daemon-resolved".into(),
        compatibility_entry: "daemon-resolved".into(),
    }
}
