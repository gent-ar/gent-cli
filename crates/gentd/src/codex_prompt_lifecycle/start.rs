//! Durable-before-spawn start transition for one claimed Codex prompt.

use std::collections::BTreeMap;

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
    let start = super::launch::LaunchSetup::read(runtime, &prompt)?;
    if runner.has_codex_session(&run_id) {
        return submit(runtime, runner, coordinator_id, active, prompt, host_epoch);
    }
    let fresh_context = runtime.launch_context(&prompt.message)?;
    let upgraded =
        fresh_context.is_none() && !runtime.runs().launched_executable_is_current(&run_id)?;
    runner.prepare_codex_prompt(
        run_id.clone(),
        start.start(runtime, &prompt, fresh_context)?,
    )?;
    if let Err(error) = runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch) {
        runner.cancel_codex_prompt(&run_id);
        return Err(error);
    }
    let launch = request(&run_id, coordinator_id, host_epoch);
    let response = match runtime.runs().start_or_resume(launch) {
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
                    steers: Vec::new(),
                    upgraded,
                },
            );
            Ok(CodexPromptDispatchOutcome::Started { run_id })
        }
        PublicRunOutcome::Denied | PublicRunOutcome::LeaseContended => {
            runner.cancel_codex_prompt(&run_id);
            runtime.release_unstarted_prompt_launch(&message_id, coordinator_id, host_epoch)?;
            Ok(CodexPromptDispatchOutcome::Empty)
        }
        PublicRunOutcome::Interrupted => {
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
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader
        + gent_ports::AttachmentLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    let goal = runtime.active_goal_for(&prompt.message.conversation_id)?;
    let (prompt_text, attachments) =
        super::launch::provider_input(runtime, &prompt.message, &run_id)?;
    let interrupted_reply = runtime.interrupted_reply_before(&prompt.message)?;
    runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch)?;
    if let Err(error) = runner.submit_codex_prompt(
        &run_id,
        &prompt_text,
        goal.as_ref(),
        &attachments,
        interrupted_reply.as_deref(),
    ) {
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

pub(super) fn request(
    run_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
) -> PublicRunStartRequest {
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
