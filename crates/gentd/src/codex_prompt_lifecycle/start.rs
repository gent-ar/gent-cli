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
        + gent_ports::AgentChatWorkspaceLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    let workspace = runtime.workspace_for_run(&prompt.message.conversation_id, &run_id)?;
    let working_directory = workspace.canonical_path;
    let workspace_root = std::path::PathBuf::from(&working_directory);
    let fresh_context = runtime
        .contexts
        .fresh_context_for_child(&prompt.message.conversation_id, &run_id)?;
    let selection = runtime.selection_for_run(&prompt.message.conversation_id, &run_id)?;
    let turn_options = gent_drivers::codex_session::CodexTurnOptions::from_selection(
        &selection,
        Some(&working_directory),
    )
    .map_err(|error| gent_ports::PublicProviderRunError::Failed(error.to_string()))?;
    if runner.has_codex_session(&run_id) {
        return submit(runtime, runner, coordinator_id, active, prompt, host_epoch);
    }
    let goal = runtime.active_goal_for(&prompt.message.conversation_id, &run_id)?;
    if let Err(error) = runner.prepare_codex_prompt(
        run_id.clone(),
        CodexPromptStart {
            working_directory: Some(working_directory),
            workspace_root,
            workspace_access: gent_types::SandboxWorkspaceAccess::from_mode(selection.mode),
            prompt: prompt.message.text.clone(),
            goal,
            fresh_context: fresh_context.clone(),
            turn_options,
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
        + gent_ports::AgentChatReadLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    let goal = runtime.active_goal_for(&prompt.message.conversation_id, &run_id)?;
    runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch)?;
    if let Err(error) = runner.submit_codex_prompt(&run_id, &prompt.message.text, goal.as_ref()) {
        runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
        return Err(error.into());
    }
    if let Err(error) = runtime.confirm_prompt_started(&message_id, coordinator_id, host_epoch) {
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
        },
    );
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
