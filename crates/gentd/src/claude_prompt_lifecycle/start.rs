//! Durable-before-spawn transition for one claimed Claude prompt.

use std::collections::BTreeMap;

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, RunProjectionLedger,
    TranscriptLedger,
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
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    let fresh_context = runtime
        .contexts
        .fresh_context_for_child(&prompt.message.conversation_id, &run_id)?;
    let goal = runtime.active_goal_for(&prompt.message.conversation_id, &run_id)?;
    if let Err(error) = runner.prepare_claude_prompt(
        run_id.clone(),
        ClaudePromptStart {
            prompt: prompt.message.text.clone(),
            goal,
            fresh_context: fresh_context.clone(),
        },
    ) {
        runtime.release_prompt_claim(&message_id, coordinator_id, host_epoch)?;
        return Err(error.into());
    }
    if let Err(error) = runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch) {
        runner.cancel_claude_prompt(&run_id);
        return Err(error);
    }
    let request = request(&run_id, coordinator_id, host_epoch);
    match if fresh_context.is_some() {
        runtime.runs().start(request)
    } else {
        runtime.runs().start_or_resume(request)
    } {
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
                    },
                );
                Ok(ClaudePromptDispatchOutcome::Started { run_id })
            }
            PublicRunOutcome::Denied | PublicRunOutcome::LeaseContended => {
                runner.cancel_claude_prompt(&run_id);
                runtime.release_unstarted_prompt_launch(&message_id, coordinator_id, host_epoch)?;
                Ok(ClaudePromptDispatchOutcome::Empty)
            }
            PublicRunOutcome::ProviderChanged | PublicRunOutcome::Interrupted => {
                runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
                Ok(ClaudePromptDispatchOutcome::Unprovable { run_id })
            }
        },
    }
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
