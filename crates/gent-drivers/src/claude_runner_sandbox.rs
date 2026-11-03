use super::{ClaudeStreamRunner, ProcessLauncher, ProviderProcess};
use crate::interrupt::ProcessTreeControl;

impl<L, P> ClaudeStreamRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub fn signal(
        &self,
        run_id: &str,
        signal: super::ProcessTreeSignal,
    ) -> Result<(), super::ClaudeRunnerError> {
        self.runs
            .get(run_id)
            .ok_or(super::ClaudeRunnerError::NotActive)?
            .process
            .signal_tree(signal)?;
        Ok(())
    }

    /// Returns true only while this owner retains the live process.
    pub fn is_live(&self, run_id: &str) -> bool {
        self.runs
            .get(run_id)
            .is_some_and(|run| matches!(run.process.try_exit_code(), Ok(None)))
    }

    pub fn prepare_permission_binding(
        &mut self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), super::ClaudeRunnerError> {
        self.launcher.validate_permission_binding(&binding)?;
        if !self.runs.contains_key(binding.run_id()) {
            self.permission_bindings
                .insert(binding.run_id().into(), binding);
        }
        Ok(())
    }

    pub fn sandbox_evidence(&self, run_id: &str) -> Option<&crate::ProcessSandboxEvidence> {
        let process = &self.runs.get(run_id)?.process;
        if process.try_exit_code().ok()?.is_some() {
            return None;
        }
        process
            .sandbox_evidence()
            .filter(|proof| proof.run_id() == run_id)
    }
}

/// Verify the actual owned process before the first provider protocol frame.
pub(super) fn launch_checked<L: ProcessLauncher>(
    launcher: &L,
    launch: &crate::supervisor::ProviderLaunch,
) -> Result<L::Process, super::ClaudeRunnerError> {
    let required = launcher.requires_sandbox_evidence(launch);
    let process = launcher.launch(launch)?;
    let valid = match process.sandbox_evidence() {
        Some(proof) => {
            proof.matches_provider_launch(launch) && matches!(process.try_exit_code(), Ok(None))
        }
        None => !required,
    };
    if !valid {
        let _ = process.signal_tree(super::ProcessTreeSignal::Terminate);
        return Err(crate::supervisor::SupervisorError::Launch(
            "Claude native process evidence does not match its live run".into(),
        )
        .into());
    }
    Ok(process)
}

#[cfg(test)]
#[path = "claude_runner_contained_tests.rs"]
mod contained_tests;
