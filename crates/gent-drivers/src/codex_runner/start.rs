use std::collections::BTreeMap;
use std::path::PathBuf;

use gent_types::{RunVersionLock, SandboxWorkspaceAccess};

use super::output::write;
use super::{
    CodexAppServerRunner, CodexRunStart, CodexRunnerError, OwnedRun, ProcessLauncher,
    ProviderProcess, sandbox,
};
use crate::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use crate::lock::recheck;
use crate::output_pump::{MAX_OUTPUT_CHUNK_BYTES, MAX_PROVIDER_FRAME_BYTES, ProviderOutputPump};
use crate::supervisor::{LaunchIntent, ProviderLaunch};

impl<L, P> CodexAppServerRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    pub fn start(&mut self, start: CodexRunStart) -> Result<(), CodexRunnerError> {
        self.admit(&start)?;
        let (turn, initial) = CodexTurnDriver::start_with_attachments(
            start.session,
            &start.prompt,
            start.attachments,
            start.goal.as_ref(),
            start.interrupted_reply,
        )?;
        self.launch(
            start.run_id,
            &start.lock,
            (start.workspace_root, start.workspace_access),
            turn,
            initial,
        )
    }

    pub fn prewarm(&mut self, start: CodexRunStart) -> Result<(), CodexRunnerError> {
        self.admit(&start)?;
        let (turn, initial) = CodexTurnDriver::awaiting_prompt(start.session)?;
        self.launch(
            start.run_id,
            &start.lock,
            (start.workspace_root, start.workspace_access),
            turn,
            initial,
        )
    }

    fn admit(&self, start: &CodexRunStart) -> Result<(), CodexRunnerError> {
        if self.runs.contains_key(&start.run_id) {
            return Err(CodexRunnerError::AlreadyActive);
        }
        if start.lock.provider != "codex" {
            return Err(CodexRunnerError::UnsupportedProvider);
        }
        Ok(())
    }

    fn launch(
        &mut self,
        run_id: String,
        lock: &RunVersionLock,
        (workspace_root, workspace_access): (PathBuf, SandboxWorkspaceAccess),
        mut turn: CodexTurnDriver,
        initial: Vec<CodexTurnEffect>,
    ) -> Result<(), CodexRunnerError> {
        let output = ProviderOutputPump::new(
            MAX_OUTPUT_CHUNK_BYTES,
            MAX_PROVIDER_FRAME_BYTES,
            self.policy,
        )?;
        recheck(lock)?;
        let launch = ProviderLaunch {
            permission_binding: self.permission_bindings.remove(&run_id),
            run_id: Some(run_id.clone()),
            lock: lock.clone(),
            provider: "codex".into(),
            executable: PathBuf::from(&lock.canonical_path),
            arguments: crate::launch_spec::codex_app_server_arguments(),
            intent: LaunchIntent::Start,
            workspace_root: Some(workspace_root),
            workspace_access,
        };
        let require_evidence = self.launcher.requires_sandbox_evidence(&launch);
        let process = self.launcher.launch(&launch)?;
        if let Err(error) =
            sandbox::bind_external_process(&process, &launch, require_evidence, &mut turn)
        {
            let _ = process.signal_tree(crate::interrupt::ProcessTreeSignal::Terminate);
            return Err(error);
        }
        for effect in initial {
            if let Err(error) = write(&process, effect) {
                let _ = process.signal_tree(crate::interrupt::ProcessTreeSignal::Terminate);
                return Err(error);
            }
        }
        self.runs.insert(
            run_id,
            OwnedRun {
                process,
                turn,
                output,
                controls: BTreeMap::new(),
            },
        );
        Ok(())
    }
}
