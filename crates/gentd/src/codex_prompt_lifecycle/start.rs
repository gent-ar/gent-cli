//! Durable-before-spawn start transition for one claimed Codex prompt.

use std::collections::BTreeMap;

use gent_drivers::codex_prompt_runner::CodexPromptStart;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    RunProjectionLedger, TranscriptLedger,
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
    working_directory: Option<&str>,
    active: &mut BTreeMap<String, Binding>,
    prompt: AgentChatPromptSaved,
    host_epoch: HostEpoch,
) -> Result<CodexPromptDispatchOutcome, RuntimeError>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    if runner.has_codex_session(&run_id) {
        return submit(runtime, runner, coordinator_id, active, prompt, host_epoch);
    }
    if let Err(error) = runner.prepare_codex_prompt(
        run_id.clone(),
        CodexPromptStart {
            working_directory: working_directory.map(str::to_owned),
            prompt: prompt.message.text.clone(),
        },
    ) {
        runtime.release_prompt_claim(&message_id, coordinator_id, host_epoch)?;
        return Err(error.into());
    }
    if let Err(error) = runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch) {
        runner.cancel_codex_prompt(&run_id);
        return Err(error);
    }
    let response = match runtime
        .runs()
        .start(request(&run_id, coordinator_id, host_epoch))
    {
        Ok(response) => response,
        Err(error) => {
            runtime.mark_prompt_unprovable(&message_id, coordinator_id, host_epoch)?;
            return Err(error);
        }
    };
    match response.outcome {
        PublicRunOutcome::Started => {
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
                },
            );
            Ok(CodexPromptDispatchOutcome::Started { run_id })
        }
        PublicRunOutcome::Denied | PublicRunOutcome::LeaseContended => {
            runner.cancel_codex_prompt(&run_id);
            runtime.release_unstarted_prompt_launch(&message_id, coordinator_id, host_epoch)?;
            Ok(CodexPromptDispatchOutcome::Empty)
        }
        PublicRunOutcome::ProviderChanged
        | PublicRunOutcome::Interrupted
        | PublicRunOutcome::Resumed => {
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
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    let run_id = prompt.run_id.0.clone();
    let message_id = prompt.message.message_id.clone();
    runtime.begin_prompt_launch(&message_id, coordinator_id, host_epoch)?;
    if let Err(error) = runner.submit_codex_prompt(&run_id, &prompt.message.text) {
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
