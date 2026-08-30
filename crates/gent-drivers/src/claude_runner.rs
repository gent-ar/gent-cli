use crate::PublicProvider;
use crate::buffering::BufferPolicy;
use crate::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
use crate::claude_permission_relay::ClaudePermissionRelay;
use crate::claude_tool_results;
use crate::claude_turn_options::ClaudeTurnOptions;
use crate::interrupt::ProcessTreeSignal;
use crate::launch_spec::{LaunchIntent, append_claude_mcp_config, arguments};
use crate::lock::{LockError, recheck};
use crate::output_pump::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
use crate::public_protocol::{PublicWireFact, claude_protocol, normalize_public_frame};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::{
    FrozenConversationContext, GoalProjection, NormalizedProviderEvent, RunVersionLock,
    SandboxWorkspaceAccess,
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
    pub resume_session_id: Option<String>,
    pub workspace_root: PathBuf,
    pub workspace_access: SandboxWorkspaceAccess,
    pub mcp_config: Option<PathBuf>,
    pub selected_mcp_source_names: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaudeRunnerEffect {
    Fact(PublicWireFact),
    PermissionRequest(ClaudePermissionRequest),
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
    launcher: L,
    policy: BufferPolicy,
    runs: BTreeMap<String, OwnedRun<P>>,
}

#[derive(Debug)]
struct OwnedRun<P> {
    process: P,
    output: ProviderOutputPump,
    permissions: ClaudePermissionRelay,
    tool_names: BTreeMap<String, String>,
    child_ids: BTreeMap<String, String>,
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
            launcher,
            policy,
            runs: BTreeMap::new(),
        }
    }

    /// Rechecks and launches one locked Claude binary before writing its exact prompt frame.
    ///
    /// # Errors
    /// Returns before launch for invalid input or a changed immutable executable lock.
    pub fn start(&mut self, start: ClaudeRunStart) -> Result<(), ClaudeRunnerError> {
        if self.runs.contains_key(&start.run_id) {
            return Err(ClaudeRunnerError::AlreadyActive);
        }
        if start.lock.provider != "claude" {
            return Err(ClaudeRunnerError::UnsupportedProvider);
        }
        let input = input_frame(&start)?;
        let output =
            ProviderOutputPump::new(MAX_OUTPUT_CHUNK_BYTES, MAX_CLAUDE_FRAME_BYTES, self.policy)?;
        recheck(&start.lock)?;
        let intent = start
            .resume_session_id
            .as_ref()
            .map_or(LaunchIntent::Start, |session_id| LaunchIntent::Resume {
                session_id: session_id.clone(),
            });
        let mut arguments = arguments("claude", &intent)
            .map_err(|error| SupervisorError::Launch(error.to_string()))?;
        start.turn_options.append_arguments(&mut arguments);
        append_claude_mcp_config(&mut arguments, start.mcp_config.as_deref());
        let launch = ProviderLaunch {
            lock: start.lock.clone(),
            provider: "claude".into(),
            executable: PathBuf::from(&start.lock.canonical_path),
            arguments,
            intent,
            workspace_root: Some(start.workspace_root),
            workspace_access: start.workspace_access,
        };
        let process = self.launcher.launch(&launch)?;
        if let Err(error) = process.write_frame(&input) {
            let _ = process.signal_tree(ProcessTreeSignal::Terminate);
            return Err(error.into());
        }
        self.runs.insert(
            start.run_id,
            OwnedRun {
                process,
                output,
                permissions: ClaudePermissionRelay::default(),
                tool_names: BTreeMap::new(),
                child_ids: BTreeMap::new(),
            },
        );
        Ok(())
    }

    /// Sends a later user turn to the process that already owns this conversation.
    pub fn submit(
        &mut self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
        content: &[serde_json::Value],
    ) -> Result<(), ClaudeRunnerError> {
        let input = input::follow_up_input_frame(prompt, goal, content)?;
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?;
        run.process.write_frame(&input)?;
        Ok(())
    }

    #[must_use]
    pub fn owns(&self, run_id: &str) -> bool {
        self.runs.contains_key(run_id)
    }

    /// Terminates and forgets an idle session so another conversation can use
    /// the bounded provider-process capacity.
    pub fn release(&mut self, run_id: &str) -> Result<(), ClaudeRunnerError> {
        let run = self.runs.get(run_id).ok_or(ClaudeRunnerError::NotActive)?;
        run.process.signal_tree(ProcessTreeSignal::Terminate)?;
        self.runs.remove(run_id);
        Ok(())
    }

    /// Drains one process chunk or a fully drained terminal exit into normalized public facts.
    ///
    /// # Errors
    /// Returns a controlled process or bounded framing failure without exposing raw output.
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
            return Ok(drain(run));
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

    /// Signals exactly the process tree owned by this run.
    ///
    /// # Errors
    /// Returns an error when the run is absent or its process tree cannot be signaled.
    pub fn signal(&self, run_id: &str, signal: ProcessTreeSignal) -> Result<(), ClaudeRunnerError> {
        self.runs
            .get(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?
            .process
            .signal_tree(signal)?;
        Ok(())
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
            .ok_or(ClaudeRunnerError::PermissionRequestNotPending)?;
        run.process.write_frame(&response)?;
        run.permissions.settle(request_id);
        Ok(())
    }
}

fn drain<P: ProviderProcess>(run: &mut OwnedRun<P>) -> Option<Vec<ClaudeRunnerEffect>> {
    let mut effects = Vec::new();
    while run.output.queued_frames() > 0 {
        let (frame, _) = run.output.take_frame();
        let Some(frame) = frame else { break };
        effects.extend(normalize(run, &frame));
    }
    (!effects.is_empty()).then_some(effects)
}

fn normalize<P: ProviderProcess>(run: &mut OwnedRun<P>, raw: &[u8]) -> Vec<ClaudeRunnerEffect> {
    let frame: serde_json::Value = match serde_json::from_slice(raw) {
        Ok(frame) => frame,
        Err(_) => return diagnostic("malformedClaudeFrame"),
    };
    if frame.get("type").and_then(serde_json::Value::as_str) == Some("control_request") {
        return match run.permissions.accept(&frame) {
            Ok(request) => vec![ClaudeRunnerEffect::PermissionRequest(request)],
            Err(classification) => diagnostic(classification),
        };
    }
    if frame.get("type").and_then(serde_json::Value::as_str) == Some("control_cancel_request") {
        run.permissions.cancel(&frame);
        return Vec::new();
    }
    if frame.get("type").and_then(serde_json::Value::as_str) == Some("user") {
        let mut facts = claude_tool_results::results(&mut run.tool_names, &frame)
            .unwrap_or_else(|| normalize_public_frame(PublicProvider::Claude, &frame));
        background::remember_launches(run, &frame, &mut facts);
        background::append_terminals(run, &frame, &mut facts);
        return facts.into_iter().map(ClaudeRunnerEffect::Fact).collect();
    }
    if let Some(facts) = claude_protocol::correlated_background_activity(&run.tool_names, &frame) {
        return facts.into_iter().map(ClaudeRunnerEffect::Fact).collect();
    }
    let mut facts = normalize_public_frame(PublicProvider::Claude, &frame);
    claude_tool_results::remember(&facts, &mut run.tool_names);
    background::append_terminals(run, &frame, &mut facts);
    facts.into_iter().map(ClaudeRunnerEffect::Fact).collect()
}
fn diagnostic(classification: &str) -> Vec<ClaudeRunnerEffect> {
    vec![ClaudeRunnerEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))]
}
#[path = "claude_runner_background.rs"]
mod background;
#[path = "claude_runner_input.rs"]
mod input;
use input::input_frame;
#[cfg(test)]
#[path = "claude_runner_tests.rs"]
mod tests;
