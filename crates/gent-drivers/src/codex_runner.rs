//! Bounded daemon-owned Codex app-server process runner.
//!
//! This owns process I/O only. Callers receive normalized facts and terminal settlement, never
//! provider frames or native identities. Daemon authority and durable writes remain outside it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gent_types::{GoalProjection, RunVersionLock};

use crate::buffering::BufferPolicy;
use crate::codex_session::CodexSessionConfig;
use crate::codex_turn::{CodexTurnDriver, CodexTurnEffect, CodexTurnError, MAX_CODEX_FRAME_BYTES};
use crate::lock::{LockError, recheck};
use crate::output_pump::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
use crate::public_protocol::PublicWireFact;
use crate::supervisor::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError,
};

/// Inputs for one locked Codex process and its first durable prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexRunStart {
    pub run_id: String,
    pub lock: RunVersionLock,
    pub session: CodexSessionConfig,
    pub prompt: String,
    /// Optional active goal copied from the Gent ledger, never from a provider or client frame.
    pub goal: Option<GoalProjection>,
}

/// One provider-neutral fact or final process settlement from a Codex process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexRunnerEffect {
    Fact(PublicWireFact),
    Exited { code: Option<i32> },
}

/// Controlled process, framing, or correlation failure at the Codex runner boundary.
#[derive(Debug, thiserror::Error)]
pub enum CodexRunnerError {
    #[error("Codex runner already owns the run")]
    AlreadyActive,
    #[error("Codex runner does not own the requested run")]
    NotActive,
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
}

/// One synchronous owner for bounded Codex app-server processes.
#[derive(Debug)]
pub struct CodexAppServerRunner<L, P> {
    launcher: L,
    policy: BufferPolicy,
    runs: BTreeMap<String, OwnedRun<P>>,
}

impl<L, P> CodexAppServerRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    /// Creates an inert process owner. Provider discovery and authorization do not occur here.
    #[must_use]
    pub fn new(launcher: L, policy: BufferPolicy) -> Self {
        Self {
            launcher,
            policy,
            runs: BTreeMap::new(),
        }
    }

    /// Rechecks, launches, and handshakes exactly one locked Codex process for a durable prompt.
    ///
    /// # Errors
    /// Returns before launch for invalid inputs or a changed binary; no fallback executable exists.
    pub fn start(&mut self, start: CodexRunStart) -> Result<(), CodexRunnerError> {
        if self.runs.contains_key(&start.run_id) {
            return Err(CodexRunnerError::AlreadyActive);
        }
        if start.lock.provider != "codex" {
            return Err(CodexRunnerError::UnsupportedProvider);
        }
        let (turn, initial) =
            CodexTurnDriver::start(start.session, &start.prompt, start.goal.as_ref())?;
        let output =
            ProviderOutputPump::new(MAX_OUTPUT_CHUNK_BYTES, MAX_CODEX_FRAME_BYTES, self.policy)?;
        recheck(&start.lock)?;
        let launch = ProviderLaunch {
            provider: "codex".into(),
            executable: PathBuf::from(&start.lock.canonical_path),
            arguments: vec!["app-server".into()],
            intent: LaunchIntent::Start,
        };
        let process = self.launcher.launch(&launch)?;
        for effect in initial {
            if let Err(error) = write(&process, effect) {
                let _ = process.signal_tree(crate::interrupt::ProcessTreeSignal::Terminate);
                return Err(error);
            }
        }
        self.runs.insert(
            start.run_id,
            OwnedRun {
                process,
                turn,
                output,
            },
        );
        Ok(())
    }

    /// Drains at most one process chunk (or a settled exit) through bounded framing and reduction.
    ///
    /// # Errors
    /// Returns a controlled error when the owned process, frame pump, or strict turn bridge fails.
    pub fn poll(
        &mut self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, CodexRunnerError> {
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(CodexRunnerError::NotActive)?;
        if let Some(chunk) = run.process.next_stdout_chunk()? {
            run.output.accept_chunk(&chunk)?;
            return drain(run);
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

    /// Writes one later prompt and its freshly ledger-resolved active goal after the prior turn.
    ///
    /// # Errors
    /// Returns a controlled error if the run is absent, its prior turn is still active, or the
    /// owned process rejects the bounded next-turn frame.
    pub fn submit_turn(
        &mut self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
    ) -> Result<(), CodexRunnerError> {
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(CodexRunnerError::NotActive)?;
        for effect in run.turn.submit(prompt, goal)? {
            write(&run.process, effect)?;
        }
        Ok(())
    }

    /// Requests cooperative interruption of the live turn before process-tree escalation.
    ///
    /// # Errors
    /// Returns a controlled error if the owned session has no live Codex turn.
    pub fn interrupt_turn(&mut self, run_id: &str) -> Result<(), CodexRunnerError> {
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(CodexRunnerError::NotActive)?;
        write(&run.process, run.turn.interrupt()?)
    }

    /// Reports whether this process owner still owns the named native session.
    #[must_use]
    pub fn owns(&self, run_id: &str) -> bool {
        self.runs.contains_key(run_id)
    }

    /// Sends one explicit tree signal to an owned process; scheduling remains daemon-owned.
    ///
    /// # Errors
    /// Returns an error when the run is absent or the whole process tree cannot be signaled.
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
}

fn drain<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
) -> Result<Option<Vec<CodexRunnerEffect>>, CodexRunnerError> {
    let mut effects = Vec::new();
    while run.output.queued_frames() > 0 {
        let (frame, _) = run.output.take_frame();
        let Some(frame) = frame else { break };
        for effect in run.turn.receive(&frame)? {
            match effect {
                CodexTurnEffect::Write(frame) => run.process.write_frame(&frame)?,
                CodexTurnEffect::Fact(fact) => effects.push(CodexRunnerEffect::Fact(fact)),
            }
        }
    }
    Ok((!effects.is_empty()).then_some(effects))
}

fn write<P: ProviderProcess>(process: &P, effect: CodexTurnEffect) -> Result<(), CodexRunnerError> {
    if let CodexTurnEffect::Write(frame) = effect {
        process.write_frame(&frame)?;
    }
    Ok(())
}
