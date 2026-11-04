//! `PublicProviderRunner` adapter that binds a durable prompt before a locked Codex launch.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use gent_ports::PublicProviderRunError;
use gent_types::{FrozenConversationContext, GoalProjection, SandboxWorkspaceAccess};

use crate::buffering::BufferPolicy;
use crate::codex_control::CodexControlDecision;
use crate::codex_runner::{CodexAppServerRunner, CodexRunnerEffect, CodexRunnerError};
use crate::codex_session::CodexTurnOptions;
use crate::supervisor::{ProcessLauncher, ProviderProcess};
mod launch;
pub use launch::fresh_prompt;
mod mcp;
mod sandbox;
/// Prompt fields held only between daemon dispatch claim and locked process launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexPromptStart {
    pub working_directory: Option<String>,
    pub workspace_root: PathBuf,
    pub workspace_access: SandboxWorkspaceAccess,
    pub prompt: String,
    /// Optional active goal copied from the Gent ledger before durable run dispatch.
    pub goal: Option<GoalProjection>,
    /// Ledger-frozen history for a fresh session, never provider-native session state.
    pub fresh_context: Option<FrozenConversationContext>,
    pub turn_options: CodexTurnOptions,
    pub attachments: Vec<serde_json::Value>,
    pub mcp_servers: Option<serde_json::Value>,
    pub interrupted_reply: Option<String>,
}

/// Bridges durable run reservation to the bounded Codex app-server runner.
#[derive(Debug)]
pub struct CodexPromptRunner<L, P> {
    runner: Arc<Mutex<CodexAppServerRunner<L, P>>>,
    pending: Arc<Mutex<BTreeMap<String, CodexPromptStart>>>,
    mcp_servers: Option<serde_json::Value>,
    mcp_config: Option<PathBuf>,
    launched_mcp: Arc<Mutex<BTreeMap<String, (String, serde_json::Value)>>>,
    pending_mcp_conversations: Arc<Mutex<BTreeMap<String, String>>>,
    prewarmed: Arc<Mutex<std::collections::BTreeSet<String>>>,
    mcp_digests: Arc<Mutex<BTreeMap<String, String>>>,
}
impl<L, P> Clone for CodexPromptRunner<L, P> {
    fn clone(&self) -> Self {
        Self {
            runner: Arc::clone(&self.runner),
            pending: Arc::clone(&self.pending),
            mcp_servers: self.mcp_servers.clone(),
            mcp_config: self.mcp_config.clone(),
            launched_mcp: Arc::clone(&self.launched_mcp),
            pending_mcp_conversations: Arc::clone(&self.pending_mcp_conversations),
            prewarmed: Arc::clone(&self.prewarmed),
            mcp_digests: Arc::clone(&self.mcp_digests),
        }
    }
}
impl<L, P> CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    /// Creates a no-process runner with no pending prompt state.
    #[must_use]
    pub fn new(
        launcher: L,
        policy: BufferPolicy,
        mcp_servers: Option<serde_json::Value>,
        mcp_config: Option<PathBuf>,
    ) -> Self {
        Self {
            runner: Arc::new(Mutex::new(CodexAppServerRunner::new(launcher, policy))),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
            mcp_servers,
            mcp_config,
            launched_mcp: Arc::new(Mutex::new(BTreeMap::new())),
            pending_mcp_conversations: Arc::new(Mutex::new(BTreeMap::new())),
            prewarmed: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
            mcp_digests: Arc::new(Mutex::new(BTreeMap::new())),
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
        lock(&self.prewarmed).remove(run_id);
        lock(&self.pending_mcp_conversations).remove(run_id);
    }
    /// Polls one owned Codex process without exposing provider-native protocol data.
    ///
    /// # Errors
    /// Returns a controlled error when the process, frame boundary, or session contract fails.
    pub fn poll(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError> {
        let result = lock(&self.runner).poll(run_id).map_err(map_error);
        if result
            .as_ref()
            .ok()
            .and_then(|effects| effects.as_ref())
            .is_some_and(|effects| {
                effects
                    .iter()
                    .any(|effect| matches!(effect, CodexRunnerEffect::Exited { .. }))
            })
        {
            lock(&self.launched_mcp).remove(run_id);
        }
        result
    }
    pub fn submit(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[serde_json::Value],
        interrupted_reply: Option<&str>,
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .submit_turn(run_id, prompt, goal, attachments, interrupted_reply)
            .map_err(map_error)
    }
    #[must_use]
    pub fn owns(&self, run_id: &str) -> bool {
        lock(&self.runner).owns(run_id)
    }

    pub fn signal_process(
        &self,
        run_id: &str,
        signal: crate::interrupt::ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner).signal(run_id, signal).map_err(map_error)
    }

    pub fn steer_turn(
        &self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        attachments: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .steer_turn(run_id, message_id, prompt, attachments)
            .map_err(map_error)
    }

    pub fn interrupt_turn(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        lock(&self.runner).interrupt_turn(run_id).map_err(map_error)
    }

    pub fn refresh_mcp_config(&self, run_id: &str) -> Result<bool, PublicProviderRunError> {
        let Some((_, digest)) = self.current_mcp_servers()? else {
            return Ok(false);
        };
        let mut digests = lock(&self.mcp_digests);
        if digests
            .get(run_id)
            .is_none_or(|previous| previous == &digest)
        {
            return Ok(false);
        }
        digests.remove(run_id);
        lock(&self.launched_mcp).remove(run_id);
        let mut runner = lock(&self.runner);
        if !runner.owns(run_id) {
            return Ok(false);
        }
        runner.terminate(run_id).map_err(map_error)?;
        Ok(true)
    }

    pub fn release_session(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        lock(&self.launched_mcp).remove(run_id);
        lock(&self.runner).terminate(run_id).map_err(map_error)
    }

    pub fn respond_control(
        &self,
        run_id: &str,
        request_id: &str,
        decision: CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .respond_control(run_id, request_id, decision, answers)
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
        CodexRunnerError::Launch(error) => crate::run_runner::public_launch_error(error),
        other => PublicProviderRunError::Failed(other.to_string()),
    }
}
