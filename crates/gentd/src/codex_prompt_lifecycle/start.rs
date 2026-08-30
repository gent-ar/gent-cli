//! Durable-before-spawn start transition for one claimed Codex prompt.

use std::collections::BTreeMap;

use gent_drivers::codex_prompt_runner::CodexPromptStart;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_protocol::{DependencyProvider, PublicRunOutcome, PublicRunStartRequest};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatPromptSaved, HostEpoch};

use super::{Binding, CodexPromptDispatchOutcome, CodexPromptExecution};
use crate::public_driver_runtime::PublicDriversRuntime;

pub(super) fn prompt<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    runner: &D,
    coordinator_id: &str,
    active: &mut BTreeMap<String, Binding>,
    prompt: AgentChatPromptSaved,
    host_epoch: HostEpoch,
    refresh_mcp_config: bool,
) -> Result<CodexPromptDispatchOutcome, RuntimeError>
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
    let message_id = prompt.message.message_id.clone();
    let workspace = runtime.workspace_for_run(&prompt.message.conversation_id, &run_id)?;
    let working_directory = workspace.canonical_path;
    let workspace_root = std::path::PathBuf::from(&working_directory);
    let fresh_context = if refresh_mcp_config {
        Some(runtime.contexts.fresh_context_before_message(
            &prompt.message.conversation_id,
            &prompt.message.message_id,
        )?)
    } else {
        runtime
            .contexts
            .fresh_context_for_child(&prompt.message.conversation_id, &run_id)?
    };
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
        selected_mcp_source_names.extend(["gent-automations".into(), "gent-forge".into()]);
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
    if runner.has_codex_session(&run_id) {
        return submit(runtime, runner, coordinator_id, active, prompt, host_epoch);
    }
    let goal = runtime.active_goal_for(&prompt.message.conversation_id, &run_id)?;
    let attachment_metadata = runtime
        .ledger()
        .turn_attachments(&prompt.message.turn_id)
        .map_err(|error| {
            gent_ports::PublicProviderRunError::Failed(format!(
                "turn attachments are unavailable: {error}"
            ))
        })?;
    let resolved_attachments = if attachment_metadata.is_empty() {
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
            &prompt.message.turn_id,
        )
        .map_err(gent_ports::PublicProviderRunError::Failed)?
    };
    let attachments = if resolved_attachments.is_empty() {
        Vec::new()
    } else {
        let (_, codex_attachment_root) = runtime.attachment_roots()?;
        crate::provider_attachments::codex_local_images(
            &resolved_attachments,
            &codex_attachment_root.join(&run_id),
        )
        .map_err(gent_ports::PublicProviderRunError::Failed)?
    };
    if let Err(error) = runner.prepare_codex_prompt(
        run_id.clone(),
        CodexPromptStart {
            working_directory: Some(working_directory),
            workspace_root,
            workspace_access: gent_types::SandboxWorkspaceAccess::from_mode(selection.mode),
            prompt: crate::provider_attachments::prompt_with_files(
                &prompt.message.text,
                &resolved_attachments,
            ),
            goal,
            fresh_context: fresh_context.clone(),
            turn_options,
            attachments,
            selected_mcp_source_names,
        },
    ) {
        runtime.release_prompt_claim(&message_id, coordinator_id, host_epoch)?;
        return Err(error.into());
    }
    if let Err(error) = runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch) {
        runner.cancel_codex_prompt(&run_id);
        return Err(error);
    }
    let request = request(&run_id, coordinator_id, host_epoch);
    let response = match if fresh_context.is_some() {
        runtime.runs().start(request)
    } else {
        runtime.runs().start_or_resume(request)
    } {
        Ok(response) => response,
        Err(error) => {
            runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
            return Err(error);
        }
    };
    match response.outcome {
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
                    releasing: false,
                },
            );
            Ok(CodexPromptDispatchOutcome::Started { run_id })
        }
        PublicRunOutcome::Denied | PublicRunOutcome::LeaseContended => {
            runner.cancel_codex_prompt(&run_id);
            runtime.release_unstarted_prompt_launch(&message_id, coordinator_id, host_epoch)?;
            Ok(CodexPromptDispatchOutcome::Empty)
        }
        PublicRunOutcome::ProviderChanged | PublicRunOutcome::Interrupted => {
            runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
            Ok(CodexPromptDispatchOutcome::Unprovable { run_id })
        }
    }
}

fn submit<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    runner: &D,
    coordinator_id: &str,
    active: &mut BTreeMap<String, Binding>,
    prompt: AgentChatPromptSaved,
    host_epoch: HostEpoch,
) -> Result<CodexPromptDispatchOutcome, RuntimeError>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AttachmentLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    let goal = runtime.active_goal_for(&prompt.message.conversation_id, &run_id)?;
    let attachment_metadata = runtime
        .ledger()
        .turn_attachments(&prompt.message.turn_id)
        .map_err(|error| gent_ports::PublicProviderRunError::Failed(error.to_string()))?;
    let resolved_attachments = if attachment_metadata.is_empty() {
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
            &prompt.message.turn_id,
        )
        .map_err(gent_ports::PublicProviderRunError::Failed)?
    };
    let attachments = if resolved_attachments.is_empty() {
        Vec::new()
    } else {
        let (_, codex_attachment_root) = runtime.attachment_roots()?;
        crate::provider_attachments::codex_local_images(
            &resolved_attachments,
            &codex_attachment_root.join(&run_id),
        )
        .map_err(gent_ports::PublicProviderRunError::Failed)?
    };
    runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch)?;
    let prompt_text =
        crate::provider_attachments::prompt_with_files(&prompt.message.text, &resolved_attachments);
    if let Err(error) =
        runner.submit_codex_prompt(&run_id, &prompt_text, goal.as_ref(), &attachments)
    {
        runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
        return Err(error.into());
    }
    if let Err(error) = runtime.confirm_prompt_started(&message_id, coordinator_id, host_epoch) {
        let _ = runner.interrupt(&run_id);
        runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
        return Err(error);
    }
    let binding = active
        .get_mut(&run_id)
        .ok_or_else(super::error::missing_binding)?;
    binding.prompt = prompt;
    binding.settled = false;
    binding.releasing = false;
    Ok(CodexPromptDispatchOutcome::Started { run_id })
}

fn request(run_id: &str, coordinator_id: &str, host_epoch: HostEpoch) -> PublicRunStartRequest {
    PublicRunStartRequest {
        run_id: run_id.into(),
        coordinator_id: coordinator_id.into(),
        host_epoch,
        provider: DependencyProvider::Codex,
        executable: "daemon-resolved".into(),
        version: "daemon-resolved".into(),
        compatibility_entry: "daemon-resolved".into(),
    }
}
