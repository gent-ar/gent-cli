//! Claude one-shot process adapter owned only by the dormant daemon lifecycle.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::claude_turn_options::ClaudeTurnOptions;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_drivers::supervisor::{ProcessLauncher, ProviderProcess};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::{
    FrozenConversationContext, GoalProjection, RunVersionLock, SandboxWorkspaceAccess,
};

#[path = "execution_mcp.rs"]
mod mcp;
use mcp::selected_config;
#[path = "execution_trait.rs"]
mod execution_trait;

/// Prompt held only between a durable dispatch claim and a locked Claude launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudePromptStart {
    pub(crate) workspace_root: PathBuf,
    pub(crate) workspace_access: SandboxWorkspaceAccess,
    pub(crate) prompt: String,
    pub(crate) turn_options: ClaudeTurnOptions,
    pub(crate) goal: Option<GoalProjection>,
    pub(crate) fresh_context: Option<FrozenConversationContext>,
    pub(crate) content: Vec<serde_json::Value>,
    pub(crate) selected_mcp_source_names: Vec<String>,
}

/// Binds each public-run reservation to one bounded Claude stream process.
#[derive(Debug)]
pub(crate) struct ClaudePromptRunner<L, P> {
    runner: Arc<Mutex<ClaudeStreamRunner<L, P>>>,
    pending: Arc<Mutex<BTreeMap<String, ClaudePromptStart>>>,
    selected_configs: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    mcp_config: Option<PathBuf>,
}

impl<L, P> Clone for ClaudePromptRunner<L, P> {
    fn clone(&self) -> Self {
        Self {
            runner: Arc::clone(&self.runner),
            pending: Arc::clone(&self.pending),
            selected_configs: Arc::clone(&self.selected_configs),
            mcp_config: self.mcp_config.clone(),
        }
    }
}

impl<L, P> ClaudePromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    fn cleanup_config(&self, run_id: &str) {
        if let Some(path) = lock(&self.selected_configs).remove(run_id) {
            let _ = std::fs::remove_file(path);
        }
    }

    #[must_use]
    pub(crate) fn new(
        launcher: L,
        policy: gent_drivers::buffering::BufferPolicy,
        mcp_config: Option<PathBuf>,
    ) -> Self {
        Self {
            runner: Arc::new(Mutex::new(ClaudeStreamRunner::new(launcher, policy))),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
            selected_configs: Arc::new(Mutex::new(BTreeMap::new())),
            mcp_config,
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
        let effects = lock(&self.runner).poll(run_id).map_err(map_error)?;
        if effects.as_ref().is_some_and(|values| {
            values
                .iter()
                .any(|effect| matches!(effect, ClaudeRunnerEffect::Exited { .. }))
        }) {
            self.cleanup_config(run_id);
        }
        Ok(effects)
    }

    pub(crate) fn owns(&self, run_id: &str) -> bool {
        lock(&self.runner).owns(run_id)
    }

    pub(crate) fn submit(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        content: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .submit(run_id, prompt, goal, content)
            .map_err(map_error)
    }

    pub(crate) fn release(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        lock(&self.runner).release(run_id).map_err(map_error)?;
        self.cleanup_config(run_id);
        Ok(())
    }

    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), PublicProviderRunError> {
        self.respond_permission_with_input(run_id, request_id, behavior, persist_suggestions, None)
    }

    pub(crate) fn respond_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .respond_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(map_error)
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
        let mcp_config = selected_config(
            self.mcp_config.as_deref(),
            &prompt.selected_mcp_source_names,
            run_id,
        )?;
        if let Some(path) = &mcp_config {
            if self.mcp_config.as_deref() != Some(path.as_path()) {
                lock(&self.selected_configs).insert(run_id.into(), path.clone());
            }
        }
        let result = lock(&self.runner)
            .start(ClaudeRunStart {
                run_id: run_id.into(),
                lock: lock_value.clone(),
                prompt: prompt.prompt,
                content: prompt.content,
                turn_options: prompt.turn_options,
                goal: prompt.goal,
                fresh_context: prompt.fresh_context,
                mcp_config,
                selected_mcp_source_names: Vec::new(),
                resume_session_id,
                workspace_root: prompt.workspace_root,
                workspace_access: prompt.workspace_access,
            })
            .map_err(map_error);
        if result.is_err() {
            self.cleanup_config(run_id);
        }
        result
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
    fn has_claude_session(&self, run_id: &str) -> bool;
    fn release_claude_session(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
    fn submit_claude_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        content: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError>;
    fn signal_claude_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError>;
    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), PublicProviderRunError>;

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
        let _ = updated_input;
        self.respond_claude_permission(run_id, request_id, behavior, persist_suggestions)
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
