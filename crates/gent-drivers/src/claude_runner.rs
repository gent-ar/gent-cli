//! Bounded daemon-owned Claude stream-JSON process runner.
use crate::PublicProvider;
use crate::buffering::BufferPolicy;
use crate::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
use crate::claude_permission_relay::ClaudePermissionRelay;
use crate::claude_turn_options::ClaudeTurnOptions;
use crate::goal_projection::project_prompt;
use crate::interrupt::ProcessTreeSignal;
use crate::launch_spec::{LaunchIntent, arguments};
use crate::lock::{LockError, recheck};
use crate::output_pump::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
use crate::public_protocol::{PublicWireFact, normalize_public_frame};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::{
    FrozenConversationContext, GoalProjection, NormalizedLifecycleSignal, NormalizedProviderEvent,
    RunVersionLock, SandboxWorkspaceAccess, ToolActivity, ToolPhase,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
/// Maximum retained Claude stream-JSON line accepted at this process boundary.
pub const MAX_CLAUDE_FRAME_BYTES: usize = 64 * 1024;

/// Inputs for a locked Claude process and exactly one daemon-owned user prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeRunStart {
    pub run_id: String,
    pub lock: RunVersionLock,
    pub prompt: String,
    /// Bounded model and permission fields derived from this run's durable selection.
    pub turn_options: ClaudeTurnOptions,
    /// Optional active goal copied from the Gent ledger, never from a provider or client frame.
    pub goal: Option<GoalProjection>,
    /// Gent-owned history used only for a fresh provider-native session.
    pub fresh_context: Option<FrozenConversationContext>,
    pub resume_session_id: Option<String>,
    pub workspace_root: PathBuf,
    pub workspace_access: SandboxWorkspaceAccess,
}

/// One normalized Claude fact or terminal process settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaudeRunnerEffect {
    Fact(PublicWireFact),
    /// A provider permission relay which contains only the identifiers Gent needs to decide it.
    /// Provider-native input and permission suggestions remain in the owned process boundary.
    PermissionRequest(ClaudePermissionRequest),
    Exited {
        code: Option<i32>,
    },
}

/// Controlled Claude process, framing, launch, or immutable-lock failure.
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

/// One synchronous owner for bounded Claude stream-JSON processes.
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
    /// Names are learned only from a preceding Claude tool-use start and remain process-local
    /// until their matching result is normalized.
    tool_names: BTreeMap<String, String>,
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
            },
        );
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
        let run = self
            .runs
            .get_mut(run_id)
            .ok_or(ClaudeRunnerError::NotActive)?;
        let response = run
            .permissions
            .response(request_id, behavior, persist_suggestions)
            .ok_or(ClaudeRunnerError::PermissionRequestNotPending)?;
        run.process.write_frame(&response)?;
        run.permissions.settle(request_id);
        Ok(())
    }
}

fn input_frame(start: &ClaudeRunStart) -> Result<Vec<u8>, ClaudeRunnerError> {
    if start.run_id.trim().is_empty()
        || start.prompt.trim().is_empty()
        || start.prompt.len() > MAX_CLAUDE_FRAME_BYTES
    {
        return Err(ClaudeRunnerError::InvalidPrompt);
    }
    if start.fresh_context.is_some() && start.resume_session_id.is_some() {
        return Err(ClaudeRunnerError::InvalidPrompt);
    }
    let prompt = match &start.fresh_context {
        Some(context) => crate::conversation_context_input::render_fresh_conversation_input(
            context,
            &start.prompt,
            MAX_CLAUDE_FRAME_BYTES,
        )
        .map_err(|_| ClaudeRunnerError::InvalidPrompt)?
        .prompt()
        .to_owned(),
        None => project_prompt(&start.prompt, start.goal.as_ref(), MAX_CLAUDE_FRAME_BYTES)
            .map_err(|_| ClaudeRunnerError::InvalidPrompt)?,
    };
    let prompt = if start.fresh_context.is_some() {
        project_prompt(&prompt, start.goal.as_ref(), MAX_CLAUDE_FRAME_BYTES)
            .map_err(|_| ClaudeRunnerError::InvalidPrompt)?
    } else {
        prompt
    };
    let mut value = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] },
        "parent_tool_use_id": null,
    });
    if let Some(session_id) = &start.resume_session_id {
        value["session_id"] = serde_json::Value::String(session_id.clone());
    }
    let mut frame = serde_json::to_vec(&value).map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    frame.push(b'\n');
    Ok(frame)
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
    if frame.get("type").and_then(serde_json::Value::as_str) == Some("user") {
        return tool_results(run, &frame);
    }
    let facts = normalize_public_frame(PublicProvider::Claude, &frame);
    remember_tool_names(run, &facts);
    facts.into_iter().map(ClaudeRunnerEffect::Fact).collect()
}

fn remember_tool_names<P>(run: &mut OwnedRun<P>, facts: &[PublicWireFact]) {
    for fact in facts {
        let PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) = fact
        else {
            continue;
        };
        if activity.phase == ToolPhase::Started {
            run.tool_names
                .entry(activity.tool_use_id.clone())
                .or_insert_with(|| activity.tool_name.clone());
        }
    }
}

fn tool_results<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
    frame: &serde_json::Value,
) -> Vec<ClaudeRunnerEffect> {
    let Some(content) = frame
        .pointer("/message/content")
        .and_then(serde_json::Value::as_array)
    else {
        return normalize_public_frame(PublicProvider::Claude, frame)
            .into_iter()
            .map(ClaudeRunnerEffect::Fact)
            .collect();
    };
    content
        .iter()
        .filter(|block| {
            block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
        })
        .flat_map(|block| tool_result(run, block))
        .collect()
}

fn tool_result<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
    block: &serde_json::Value,
) -> Vec<ClaudeRunnerEffect> {
    let Some(tool_use_id) = block
        .get("tool_use_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return diagnostic("malformedClaudeToolResult");
    };
    let Some(tool_name) = run.tool_names.remove(tool_use_id) else {
        return diagnostic("unresolvedClaudeToolResult");
    };
    let phase = if block.get("is_error").and_then(serde_json::Value::as_bool) == Some(true) {
        ToolPhase::Failed
    } else {
        ToolPhase::Completed
    };
    let output_digest = block
        .get("content")
        .map(|value| format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes())));
    vec![ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: tool_use_id.into(),
                tool_name,
                phase,
                output_digest,
            },
        },
    ))]
}

fn diagnostic(classification: &str) -> Vec<ClaudeRunnerEffect> {
    vec![ClaudeRunnerEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))]
}

#[cfg(test)]
#[path = "claude_runner_tests.rs"]
mod tests;
