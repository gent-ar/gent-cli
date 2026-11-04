use super::{
    ClaudeRunStart, ClaudeRunnerError, ClaudeStreamRunner, OwnedRun, ProcessTreeSignal, input,
    sandbox,
};
use crate::claude_permission_relay::ClaudePermissionRelay;
use crate::launch_spec::{LaunchIntent, append_claude_mcp_config, arguments};
use crate::lock::recheck;
use crate::output_pump::{MAX_OUTPUT_CHUNK_BYTES, MAX_PROVIDER_FRAME_BYTES, ProviderOutputPump};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use std::collections::BTreeMap;
use std::path::PathBuf;

impl<L, P> ClaudeStreamRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub fn start(&mut self, start: ClaudeRunStart) -> Result<(), ClaudeRunnerError> {
        let input = input::input_frame(&start)?;
        self.launch(start, Some(&input))
    }

    pub fn spawn_awaiting_input(&mut self, start: ClaudeRunStart) -> Result<(), ClaudeRunnerError> {
        if start.run_id.trim().is_empty() || start.fresh_context.is_some() {
            return Err(ClaudeRunnerError::InvalidPrompt);
        }
        self.launch(start, None)
    }

    fn launch(
        &mut self,
        start: ClaudeRunStart,
        input: Option<&[u8]>,
    ) -> Result<(), ClaudeRunnerError> {
        if self.runs.contains_key(&start.run_id) {
            return Err(ClaudeRunnerError::AlreadyActive);
        }
        if start.lock.provider != "claude" {
            return Err(ClaudeRunnerError::UnsupportedProvider);
        }
        let plans_for_review = start.turn_options.plans_for_review();
        let output = ProviderOutputPump::new(
            MAX_OUTPUT_CHUNK_BYTES,
            MAX_PROVIDER_FRAME_BYTES,
            self.policy,
        )?;
        recheck(&start.lock)?;
        let intent = start.intent.clone();
        let awaiting_resumed_frame = matches!(intent, LaunchIntent::Resume { .. });
        let pending_resume_session = match (&intent, input) {
            (LaunchIntent::Resume { session_id }, None) => Some(session_id.clone()),
            _ => None,
        };
        let mut arguments = arguments("claude", &intent)
            .map_err(|error| SupervisorError::Launch(error.to_string()))?;
        start.turn_options.append_arguments(&mut arguments);
        append_claude_mcp_config(&mut arguments, start.mcp_config.as_deref());
        let launch = ProviderLaunch {
            permission_binding: self.permission_bindings.remove(&start.run_id),
            run_id: Some(start.run_id.clone()),
            lock: start.lock.clone(),
            provider: "claude".into(),
            executable: PathBuf::from(&start.lock.canonical_path),
            arguments,
            intent,
            workspace_root: Some(start.workspace_root),
            workspace_access: start.workspace_access,
        };
        let process = sandbox::launch_checked(&self.launcher, &launch)?;
        if let Some(input) = input
            && let Err(error) = process.write_frame(input)
        {
            let _ = process.signal_tree(ProcessTreeSignal::Terminate);
            return Err(error.into());
        }
        self.runs.insert(
            start.run_id,
            OwnedRun {
                process,
                output,
                permissions: ClaudePermissionRelay::default(),
                started_tools: BTreeMap::new(),
                child_ids: BTreeMap::new(),
                steers: BTreeMap::new(),
                plans_for_review,
                awaiting_resumed_frame,
                pending_resume_session,
            },
        );
        Ok(())
    }
}
