//! Durable-before-spawn transition for one claimed Claude prompt.

use std::collections::BTreeMap;

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, TranscriptLedger,
};
use gent_protocol::PublicRunOutcome;
use gent_runtime::RuntimeError;
use gent_types::{AgentChatPromptSaved, HostEpoch};

use super::launch::prompt_start;
use super::{Binding, ClaudePromptDispatchOutcome, ClaudePromptExecution};
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
            start,
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
                        failure_reported_turn: None,
                        open_children: std::collections::BTreeMap::new(),
                        last_dispatch: 0,
                        idle_since: None,
                        awaits_first_input: false,
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

fn submit<L, D, R>(
    runtime: &PublicDriversRuntime<L, D, R>,
    runner: &D,
    coordinator_id: &str,
    active: &mut BTreeMap<String, Binding>,
    prompt: AgentChatPromptSaved,
    host_epoch: HostEpoch,
    start: super::ClaudePromptStart,
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
    let fresh_context = start.fresh_context.filter(|_| {
        active
            .get(&run_id)
            .is_some_and(|binding| binding.awaits_first_input)
    });
    let prompt_text = match &fresh_context {
        Some(context) => super::launch::fresh_input(context, &start.prompt)?,
        None => start.prompt,
    };
    runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch)?;
    if let Err(error) =
        runner.submit_claude_prompt(&run_id, &prompt_text, start.goal.as_ref(), &start.content)
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
    binding.awaits_first_input = false;
    Ok(ClaudePromptDispatchOutcome::Started { run_id })
}

#[path = "start_request.rs"]
mod launch_request;
pub(super) use launch_request::request;
