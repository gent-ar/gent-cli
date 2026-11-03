use super::{
    CodexPromptRunner, ProcessLauncher, ProviderProcess, PublicProviderRunError, lock, map_error,
};

impl<L, P> CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub fn prepare_permission_binding(
        &self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Result<(), PublicProviderRunError> {
        lock(&self.runner)
            .prepare_permission_binding(binding)
            .map_err(map_error)
    }

    pub fn with_sandbox_evidence<R>(
        &self,
        run_id: &str,
        inspect: impl FnOnce(Option<&crate::ProcessSandboxEvidence>) -> R,
    ) -> R {
        inspect(lock(&self.runner).sandbox_evidence(run_id))
    }
}
