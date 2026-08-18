//! `PublicProviderRunner` adapter that binds a durable prompt before a locked Codex launch.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::RunVersionLock;

use crate::buffering::BufferPolicy;
use crate::codex_runner::{
    CodexAppServerRunner, CodexRunStart, CodexRunnerEffect, CodexRunnerError,
};
use crate::codex_session::{CodexSessionConfig, CodexTurnOptions};
use crate::supervisor::{ProcessLauncher, ProviderProcess};

/// Prompt fields held only between daemon dispatch claim and locked process launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexPromptStart {
    pub working_directory: Option<String>,
    pub prompt: String,
    pub turn_options: CodexTurnOptions,
}

/// Bridges durable run reservation to the bounded Codex app-server runner.
#[derive(Clone, Debug)]
pub struct CodexPromptRunner<L, P> {
    runner: Arc<Mutex<CodexAppServerRunner<L, P>>>,
    pending: Arc<Mutex<BTreeMap<String, CodexPromptStart>>>,
}

impl<L, P> CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    /// Creates a no-process runner with no pending prompt state.
    #[must_use]
    pub fn new(launcher: L, policy: BufferPolicy) -> Self {
        Self {
            runner: Arc::new(Mutex::new(CodexAppServerRunner::new(launcher, policy))),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Preloads the one exact durable prompt that a subsequent public-run reservation may start.
    ///
    /// # Errors
    /// Rejects an attempt to overwrite an already pending prompt for the same run.
    pub fn prepare(
        &self,
        run_id: String,
        prompt: CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        if run_id.trim().is_empty() {
            return Err(PublicProviderRunError::Failed(
                "Codex run identity is invalid".into(),
            ));
        }
        let mut pending = lock(&self.pending);
        if pending.contains_key(&run_id) {
            return Err(PublicProviderRunError::Failed(
                "Codex prompt is already pending".into(),
            ));
        }
        pending.insert(run_id, prompt);
        Ok(())
    }

    /// Drops a prompt that failed before process ownership began so a daemon may release its outbox claim.
    pub fn cancel(&self, run_id: &str) {
        lock(&self.pending).remove(run_id);
    }

    /// Polls one owned Codex process without exposing provider-native protocol data.
    ///
    /// # Errors
    /// Returns a controlled error when the process, frame boundary, or session contract fails.
    pub fn poll(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError> {
        lock(&self.runner).poll(run_id).map_err(map_error)
    }

    /// Submits a later prompt on the already-owned, ready Codex native session.
    ///
    /// The daemon must durably mark its dispatch boundary before calling this method. It cannot
    /// launch, select a session, or replace the process.
    ///
    /// # Errors
    /// Returns a controlled failure when no ready owned session can accept the prompt.
    pub fn submit(&self, run_id: &str, prompt: &str) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .submit_turn(run_id, prompt)
            .map_err(map_error)
    }

    /// Reports whether the daemon-owned runner still owns the named Codex native session.
    #[must_use]
    pub fn owns(&self, run_id: &str) -> bool {
        lock(&self.runner).owns(run_id)
    }

    fn launch(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
        resume_thread_id: Option<String>,
    ) -> Result<(), PublicProviderRunError> {
        let prompt = lock(&self.pending).remove(run_id).ok_or_else(|| {
            PublicProviderRunError::Failed("Codex run has no durable pending prompt".into())
        })?;
        lock(&self.runner)
            .start(CodexRunStart {
                run_id: run_id.into(),
                lock: lock_value.clone(),
                session: CodexSessionConfig {
                    working_directory: prompt.working_directory,
                    resume_thread_id,
                    turn_options: prompt.turn_options,
                },
                prompt: prompt.prompt,
            })
            .map_err(map_error)
    }
}

impl<L, P> PublicProviderRunner for CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess,
{
    fn start(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
    ) -> Result<(), PublicProviderRunError> {
        self.launch(run_id, lock_value, None)
    }

    fn resume(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError> {
        if session_id.trim().is_empty() {
            return Err(PublicProviderRunError::Failed(
                "Codex resume thread is invalid".into(),
            ));
        }
        self.launch(run_id, lock_value, Some(session_id.into()))
    }

    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .signal(run_id, crate::interrupt::ProcessTreeSignal::Interrupt)
            .map_err(map_error)
    }
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn map_error(error: CodexRunnerError) -> PublicProviderRunError {
    match error {
        CodexRunnerError::Lock(crate::lock::LockError::ProviderChanged) => {
            PublicProviderRunError::ProviderChanged
        }
        other => PublicProviderRunError::Failed(other.to_string()),
    }
}
