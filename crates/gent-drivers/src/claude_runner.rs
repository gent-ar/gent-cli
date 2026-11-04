use crate::buffering::BufferPolicy;
use crate::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
use crate::claude_permission_relay::ClaudePermissionRelay;
use crate::claude_turn_options::ClaudeTurnOptions;
use crate::interrupt::ProcessTreeSignal;
use crate::launch_spec::LaunchIntent;
use crate::lock::LockError;
use crate::output_pump::{OutputPumpError, ProviderOutputPump};
use crate::public_protocol::PublicWireFact;
use crate::supervisor::{ProcessLauncher, ProviderProcess, SupervisorError};
use gent_types::{
    FrozenConversationContext, GoalProjection, RunVersionLock, SandboxWorkspaceAccess, ToolActivity,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
pub const MAX_CLAUDE_FRAME_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeRunStart {
    pub run_id: String,
    pub lock: RunVersionLock,
    pub prompt: String,
    pub content: Vec<serde_json::Value>,
    pub turn_options: ClaudeTurnOptions,
    pub goal: Option<GoalProjection>,
    pub fresh_context: Option<FrozenConversationContext>,
    pub intent: LaunchIntent,
    pub workspace_root: PathBuf,
    pub workspace_access: SandboxWorkspaceAccess,
    pub mcp_config: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaudeRunnerEffect {
    Fact(PublicWireFact),
    PermissionRequest(ClaudePermissionRequest),
    SteerConsumed { message_id: String },
    ResumeUnavailable,
    Exited { code: Option<i32> },
}

#[derive(Debug, thiserror::Error)]
pub enum ClaudeRunnerError {
    #[error("Claude runner already owns the run")]
    AlreadyActive,
    #[error("Claude runner does not own the requested run")]
    NotActive,
    #[error("Claude runner has no pending permission request with that identifier")]
    PermissionRequestNotPending,
    #[error("{0}")]
    InvalidPermissionInput(&'static str),
    #[error("Claude runner accepts only a locked Claude executable")]
    UnsupportedProvider,
    #[error("Claude prompt is invalid")]
    InvalidPrompt,
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Launch(#[from] SupervisorError),
    #[error(transparent)]
    Output(#[from] OutputPumpError),
    #[error(transparent)]
    Process(#[from] crate::interrupt::ProcessTreeError),
}

#[derive(Debug)]
pub struct ClaudeStreamRunner<L, P> {
    permission_bindings: BTreeMap<String, gent_types::SandboxPermissionBinding>,
    launcher: L,
    policy: BufferPolicy,
    runs: BTreeMap<String, OwnedRun<P>>,
}

#[derive(Debug)]
struct OwnedRun<P> {
    process: P,
    output: ProviderOutputPump,
    permissions: ClaudePermissionRelay,
    started_tools: BTreeMap<String, ToolActivity>,
    child_ids: BTreeMap<String, String>,
    steers: BTreeMap<String, String>,
    plans_for_review: bool,
    awaiting_resumed_frame: bool,
    pending_resume_session: Option<String>,
}

impl<L, P> ClaudeStreamRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    /// Creates an inert process owner without provider discovery or persistence.
    #[must_use]
    pub fn new(launcher: L, policy: BufferPolicy) -> Self {
        Self {
            permission_bindings: BTreeMap::new(),
            launcher,
            policy,
            runs: BTreeMap::new(),
        }
    }

    pub fn submit(
        &mut self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        content: &[serde_json::Value],
    ) -> Result<(), ClaudeRunnerError> {
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?;
        let input = match run.pending_resume_session.as_deref() {
            Some(session_id) => {
                input::resumed_first_input_frame(prompt, goal, content, session_id)?
            }
            None => input::follow_up_input_frame(prompt, goal, content)?,
        };
        run.process.write_frame(&input)?;
        run.pending_resume_session = None;
        Ok(())
    }

    pub fn steer(
        &mut self,
        run_id: &str,
        message_id: &str,
        prompt: &str,
        content: &[serde_json::Value],
    ) -> Result<(), ClaudeRunnerError> {
        let uuid = input::steer_uuid(message_id);
        let input = input::steer_input_frame(&uuid, prompt, content)?;
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?;
        run.process.write_frame(&input)?;
        run.steers.insert(uuid, message_id.into());
        Ok(())
    }

    #[must_use]
    pub fn owns(&self, run_id: &str) -> bool {
        self.runs.contains_key(run_id)
    }

    pub fn release(&mut self, run_id: &str) -> Result<(), ClaudeRunnerError> {
        let run = self.runs.get(run_id).ok_or(ClaudeRunnerError::NotActive)?;
        run.process.signal_tree(ProcessTreeSignal::Terminate)?;
        self.runs.remove(run_id);
        Ok(())
    }

    pub fn poll(
        &mut self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, ClaudeRunnerError> {
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?;
        if let Some(chunk) = run.process.next_stdout_chunk()? {
            run.output.accept_chunk(&chunk)?;
            let mut effects = drain(run).unwrap_or_default();
            // Reduce each chunk before reading more to preserve pump backpressure and FIFO.
            // A single 4 KiB read per tick otherwise delays large tool results and controls.
            for _ in 1..crate::output_pump::MAX_OUTPUT_CHUNKS_PER_POLL {
                let Some(chunk) = run.process.next_stdout_chunk()? else {
                    break;
                };
                run.output.accept_chunk(&chunk)?;
                effects.extend(drain(run).unwrap_or_default());
            }
            return Ok((!effects.is_empty()).then_some(effects));
        }
        let Some(code) = run.process.try_exit_code()? else {
            return Ok(None);
        };
        if let Some(chunk) = run.process.next_stdout_chunk()? {
            run.output.accept_chunk(&chunk)?;
            return Ok(drain(run));
        }
        self.runs.remove(run_id);
        Ok(Some(vec![ClaudeRunnerEffect::Exited { code }]))
    }

    /// Writes one closed Gent permission decision to the exact Claude process that requested it.
    ///
    /// Raw provider suggestions are retained only until this response has been accepted by the
    /// owned process. They can be echoed solely for an allowed persistent decision.
    ///
    /// # Errors
    /// Returns an error without writing when the run or pending request is absent.
    pub fn respond_permission(
        &mut self,
        run_id: &str,
        request_id: &str,
        behavior: ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), ClaudeRunnerError> {
        self.respond_permission_with_input(run_id, request_id, behavior, persist_suggestions, None)
    }

    pub fn respond_permission_with_input(
        &mut self,
        run_id: &str,
        request_id: &str,
        behavior: ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), ClaudeRunnerError> {
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?;
        let response = run
            .permissions
            .response_with_input(
                request_id,
                behavior,
                persist_suggestions,
                updated_input.as_ref(),
            )
            .map_err(ClaudeRunnerError::InvalidPermissionInput)?
            .ok_or(ClaudeRunnerError::PermissionRequestNotPending)?;
        run.process.write_frame(&response)?;
        run.permissions.settle(request_id);
        Ok(())
    }
}

#[path = "claude_runner_frames.rs"]
mod frames;
#[path = "claude_runner_input.rs"]
mod input;
#[path = "claude_runner_launch.rs"]
mod launch;
#[cfg(test)]
#[path = "claude_runner_tests.rs"]
mod tests;

use frames::drain;

#[path = "claude_runner_sandbox.rs"]
mod sandbox;
