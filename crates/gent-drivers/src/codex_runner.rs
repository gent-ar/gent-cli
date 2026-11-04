//! Bounded daemon-owned Codex app-server process runner.
//!
//! This owns process I/O only. Callers receive normalized facts and terminal settlement, never
//! provider frames or native identities. Daemon authority and durable writes remain outside it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gent_types::{GoalProjection, RunVersionLock, SandboxWorkspaceAccess};

use crate::buffering::BufferPolicy;
use crate::codex_control::{CodexControlDecision, CodexControlRequest, encode};
use crate::codex_session::CodexSessionConfig;
use crate::codex_turn::{CodexTurnDriver, CodexTurnError};
use crate::lock::LockError;
use crate::output_pump::{OutputPumpError, ProviderOutputPump};
use crate::public_protocol::PublicWireFact;
use crate::supervisor::{ProcessLauncher, ProviderProcess, SupervisorError};

mod control;
mod output;
#[path = "codex_runner_sandbox.rs"]
mod sandbox;
mod start;

use output::{drain, write};

/// Inputs for one locked Codex process and its first durable prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexRunStart {
    pub run_id: String,
    pub lock: RunVersionLock,
    pub session: CodexSessionConfig,
    pub workspace_root: PathBuf,
    pub workspace_access: SandboxWorkspaceAccess,
    pub prompt: String,
    /// Optional active goal copied from the Gent ledger, never from a provider or client frame.
    pub goal: Option<GoalProjection>,
    pub attachments: Vec<serde_json::Value>,
    pub interrupted_reply: Option<String>,
}

/// One provider-neutral fact or final process settlement from a Codex process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexRunnerEffect {
    Fact(PublicWireFact),
    ControlRequest(CodexControlRequest),
    Steer(crate::codex_session::CodexSteerOutcome),
    ResumeUnavailable,
    Exited { code: Option<i32> },
}

/// Controlled process, framing, or correlation failure at the Codex runner boundary.
#[derive(Debug, thiserror::Error)]
pub enum CodexRunnerError {
    #[error("Codex runner already owns the run")]
    AlreadyActive,
    #[error("Codex runner does not own the requested run")]
    NotActive,
    #[error("Codex control request is not pending for this run")]
    ControlNotPending,
    #[error("Codex runner accepts only a locked Codex executable")]
    UnsupportedProvider,
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Launch(#[from] SupervisorError),
    #[error(transparent)]
    Output(#[from] OutputPumpError),
    #[error(transparent)]
    Turn(#[from] CodexTurnError),
    #[error(transparent)]
    Process(#[from] crate::interrupt::ProcessTreeError),
}

#[derive(Debug)]
struct OwnedRun<P> {
    process: P,
    turn: CodexTurnDriver,
    output: ProviderOutputPump,
    controls: BTreeMap<String, CodexControlRequest>,
}

/// One synchronous owner for bounded Codex app-server processes.
#[derive(Debug)]
pub struct CodexAppServerRunner<L, P> {
    permission_bindings: BTreeMap<String, gent_types::SandboxPermissionBinding>,
    launcher: L,
    policy: BufferPolicy,
    runs: BTreeMap<String, OwnedRun<P>>,
}

impl<L, P> CodexAppServerRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub fn sandbox_evidence(&self, run_id: &str) -> Option<&crate::ProcessSandboxEvidence> {
        let process = &self.runs.get(run_id)?.process;
        if process.try_exit_code().ok()?.is_some() {
            return None;
        }
        process
            .sandbox_evidence()
            .filter(|proof| proof.run_id() == run_id)
    }

    /// Creates an inert process owner. Provider discovery and authorization do not occur here.
    #[must_use]
    pub fn new(launcher: L, policy: BufferPolicy) -> Self {
        Self {
            permission_bindings: BTreeMap::new(),
            launcher,
            policy,
            runs: BTreeMap::new(),
        }
    }

    pub fn poll(
        &mut self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, CodexRunnerError> {
        let run = self.run_mut(run_id)?;
        if let Some(chunk) = run.process.next_stdout_chunk()? {
            run.output.accept_chunk(&chunk)?;
            let mut effects = drain(run)?.unwrap_or_default();
            // Reduce each chunk before reading more to preserve pump backpressure and FIFO.
            // A single 4 KiB read per tick otherwise delays large tool results and controls.
            for _ in 1..crate::output_pump::MAX_OUTPUT_CHUNKS_PER_POLL {
                let Some(chunk) = run.process.next_stdout_chunk()? else {
                    break;
                };
                run.output.accept_chunk(&chunk)?;
                effects.extend(drain(run)?.unwrap_or_default());
            }
            return Ok((!effects.is_empty()).then_some(effects));
        }
        let Some(code) = run.process.try_exit_code()? else {
            return Ok(None);
        };
        if let Some(chunk) = run.process.next_stdout_chunk()? {
            run.output.accept_chunk(&chunk)?;
            return drain(run);
        }
        self.runs.remove(run_id);
        Ok(Some(vec![CodexRunnerEffect::Exited { code }]))
    }

    pub fn submit_turn(
        &mut self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[serde_json::Value],
        interrupted_reply: Option<&str>,
    ) -> Result<(), CodexRunnerError> {
        let run = self.run_mut(run_id)?;
        for effect in run
            .turn
            .submit(prompt, goal, attachments, interrupted_reply)?
        {
            write(&run.process, effect)?;
        }
        Ok(())
    }

    pub fn steer_turn(
        &mut self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        attachments: &[serde_json::Value],
    ) -> Result<(), CodexRunnerError> {
        let run = self.run_mut(run_id)?;
        write(
            &run.process,
            run.turn.steer(message_id, prompt, attachments)?,
        )
    }

    pub fn interrupt_turn(&mut self, run_id: &str) -> Result<(), CodexRunnerError> {
        let run = self.run_mut(run_id)?;
        write(&run.process, run.turn.interrupt()?)
    }

    pub fn respond_control(
        &mut self,
        run_id: &str,
        request_id: &str,
        decision: CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), CodexRunnerError> {
        let run = self.run_mut(run_id)?;
        let request = run
            .controls
            .remove(request_id)
            .ok_or(CodexRunnerError::ControlNotPending)?;
        run.process
            .write_frame(&encode(&request, decision, answers))?;
        Ok(())
    }

    fn run_mut(&mut self, run_id: &str) -> Result<&mut OwnedRun<P>, CodexRunnerError> {
        self.runs.get_mut(run_id).ok_or(CodexRunnerError::NotActive)
    }

    #[must_use]
    pub fn owns(&self, run_id: &str) -> bool {
        self.runs.contains_key(run_id)
    }

    pub fn signal(
        &self,
        run_id: &str,
        signal: crate::interrupt::ProcessTreeSignal,
    ) -> Result<(), CodexRunnerError> {
        self.runs
            .get(run_id)
            .ok_or(CodexRunnerError::NotActive)?
            .process
            .signal_tree(signal)?;
        Ok(())
    }

    pub fn terminate(&mut self, run_id: &str) -> Result<(), CodexRunnerError> {
        let run = self
            .runs
            .remove(run_id)
            .ok_or(CodexRunnerError::NotActive)?;
        run.process
            .signal_tree(crate::interrupt::ProcessTreeSignal::Terminate)?;
        Ok(())
    }
}
