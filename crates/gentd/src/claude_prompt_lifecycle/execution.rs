//! Claude one-shot process adapter owned only by the dormant daemon lifecycle.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::supervisor::{ProcessLauncher, ProviderProcess};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::RunVersionLock;

/// Prompt held only between a durable dispatch claim and a locked Claude launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudePromptStart {
    pub(crate) prompt: String,
}

/// Binds each public-run reservation to one bounded Claude stream process.
#[derive(Clone, Debug)]
pub(crate) struct ClaudePromptRunner<L, P> {
    runner: Arc<Mutex<ClaudeStreamRunner<L, P>>>,
    pending: Arc<Mutex<BTreeMap<String, ClaudePromptStart>>>,
}

impl<L, P> ClaudePromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    #[must_use]
    pub(crate) fn new(launcher: L, policy: gent_drivers::buffering::BufferPolicy) -> Self {
        Self {
            runner: Arc::new(Mutex::new(ClaudeStreamRunner::new(launcher, policy))),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub(crate) fn prepare(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        if run_id.trim().is_empty() || prompt.prompt.trim().is_empty() {
            return Err(PublicProviderRunError::Failed(
                "Claude prompt is invalid".into(),
            ));
        }
        let mut pending = lock(&self.pending);
        if pending.insert(run_id, prompt).is_some() {
            return Err(PublicProviderRunError::Failed(
                "Claude prompt is already pending".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn cancel(&self, run_id: &str) {
        lock(&self.pending).remove(run_id);
    }

    pub(crate) fn poll(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        lock(&self.runner).poll(run_id).map_err(map_error)
    }

    fn launch(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
        resume_session_id: Option<String>,
    ) -> Result<(), PublicProviderRunError> {
        let prompt = lock(&self.pending).remove(run_id).ok_or_else(|| {
            PublicProviderRunError::Failed("Claude run has no durable pending prompt".into())
        })?;
        lock(&self.runner)
            .start(ClaudeRunStart {
                run_id: run_id.into(),
                lock: lock_value.clone(),
                prompt: prompt.prompt,
                goal: None,
                resume_session_id,
            })
            .map_err(map_error)
    }
}

impl<L, P> PublicProviderRunner for ClaudePromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess + Send,
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
                "Claude resume session is invalid".into(),
            ));
        }
        self.launch(run_id, lock_value, Some(session_id.into()))
    }

    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .signal(run_id, ProcessTreeSignal::Interrupt)
            .map_err(map_error)
    }
}

pub(crate) trait ClaudePromptExecution: PublicProviderRunner {
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError>;
    fn cancel_claude_prompt(&self, run_id: &str);
    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError>;
}

impl<L, P> ClaudePromptExecution for ClaudePromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess + Send,
{
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.prepare(run_id, prompt)
    }

    fn cancel_claude_prompt(&self, run_id: &str) {
        self.cancel(run_id);
    }

    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        self.poll(run_id)
    }
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn map_error(error: gent_drivers::claude_runner::ClaudeRunnerError) -> PublicProviderRunError {
    match error {
        gent_drivers::claude_runner::ClaudeRunnerError::Lock(
            gent_drivers::lock::LockError::ProviderChanged,
        ) => PublicProviderRunError::ProviderChanged,
        other => PublicProviderRunError::Failed(other.to_string()),
    }
}
