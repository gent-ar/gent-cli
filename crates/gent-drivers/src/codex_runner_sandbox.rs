use super::{CodexAppServerRunner, CodexRunnerError, ProcessLauncher, ProviderProcess};

impl<L, P> CodexAppServerRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    /// Returns true only while this owner retains the live process.
    pub fn is_live(&self, run_id: &str) -> bool {
        self.runs
            .get(run_id)
            .is_some_and(|run| matches!(run.process.try_exit_code(), Ok(None)))
    }

    pub fn prepare_permission_binding(
        &mut self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), CodexRunnerError> {
        self.launcher.validate_permission_binding(&binding)?;
        if !self.runs.contains_key(binding.run_id()) {
            self.permission_bindings
                .insert(binding.run_id().into(), binding);
        }
        Ok(())
    }
}

/// External SDK policy is selected only after an actual, matching live process attestation.
pub(super) fn bind_external_process<P: ProviderProcess>(
    process: &P,
    launch: &crate::supervisor::ProviderLaunch,
    require_evidence: bool,
    turn: &mut crate::codex_turn::CodexTurnDriver,
) -> Result<(), CodexRunnerError> {
    let Some(evidence) = process.sandbox_evidence() else {
        return if require_evidence {
            Err(crate::supervisor::SupervisorError::Launch(
                "Codex native process evidence is unavailable".into(),
            )
            .into())
        } else {
            Ok(())
        };
    };
    if !evidence.matches_provider_launch(launch) || !matches!(process.try_exit_code(), Ok(None)) {
        return Err(crate::supervisor::SupervisorError::Launch(
            "Codex external sandbox evidence does not match its live run".into(),
        )
        .into());
    }
    turn.bind_external_sandbox()?;
    Ok(())
}

#[cfg(test)]
#[path = "codex_runner_external_tests.rs"]
mod external_tests;
