use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use gent_drivers::{
    PublicProvider, SystemProcess,
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    lock::capture,
    ndjson::NdjsonFramer,
    supervisor::{LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess},
};
use gent_types::SandboxWorkspaceAccess;
use serde_json::Value;

use crate::provider_launch_budget::ProviderLaunchError;

const PROBE_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const POLL_DELAY: Duration = Duration::from_millis(10);

pub(crate) enum ProbeStep<T> {
    Continue,
    Done(T),
}

pub(crate) struct ProviderProbe {
    pub(crate) provider: PublicProvider,
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) timeout: Duration,
    pub(crate) workspace_root: Option<PathBuf>,
}

impl ProviderProbe {
    pub(crate) fn exchange<T>(
        &self,
        opening_frames: &[Vec<u8>],
        mut on_frame: impl FnMut(&Value, &SystemProcess) -> Result<ProbeStep<T>, String>,
    ) -> Result<T, ProviderLaunchError> {
        let process = self.launch().map_err(ProviderLaunchError::Failed)?;
        let result = self.drive(&process, opening_frames, &mut on_frame);
        let _ = process.signal_tree(ProcessTreeSignal::Kill);
        result
    }

    fn launch(&self) -> Result<SystemProcess, String> {
        let name = self.provider.executable_name();
        let lock = capture(name, &self.executable, "unprobed", "model-catalog")
            .map_err(|_| format!("{name} executable could not be locked"))?;
        crate::node_runtime_lock::standalone_provider_launcher(PROBE_OUTPUT_BYTES)
            .launch(&ProviderLaunch {
                provider: name.into(),
                executable: PathBuf::from(&lock.canonical_path),
                lock,
                arguments: self.arguments.clone(),
                intent: LaunchIntent::Start,
                workspace_root: self.workspace_root.clone(),
                workspace_access: SandboxWorkspaceAccess::ReadOnly,
            })
            .map_err(|error| format!("{name} could not start: {error}"))
    }

    fn drive<T>(
        &self,
        process: &SystemProcess,
        opening_frames: &[Vec<u8>],
        on_frame: &mut impl FnMut(&Value, &SystemProcess) -> Result<ProbeStep<T>, String>,
    ) -> Result<T, ProviderLaunchError> {
        let name = self.provider.executable_name();
        let failed = ProviderLaunchError::Failed;
        for frame in opening_frames {
            process
                .write_frame(frame)
                .map_err(|error| failed(format!("{name} rejected the model request: {error}")))?;
        }
        let deadline = Instant::now() + self.timeout;
        let mut framer =
            NdjsonFramer::new(PROBE_OUTPUT_BYTES).map_err(|error| failed(error.to_string()))?;
        loop {
            let mut progressed = false;
            while let Some(chunk) = process
                .next_stdout_chunk()
                .map_err(|error| failed(error.to_string()))?
            {
                progressed = true;
                for line in framer.push(&chunk) {
                    let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                        continue;
                    };
                    if let ProbeStep::Done(result) = on_frame(&value, process).map_err(failed)? {
                        return Ok(result);
                    }
                }
            }
            if !progressed && process.try_exit_code().ok().flatten().is_some() {
                return Err(failed(format!("{name} exited before listing its models")));
            }
            if Instant::now() >= deadline {
                return Err(ProviderLaunchError::TimedOut(format!(
                    "{name} did not list its models in time"
                )));
            }
            if !progressed {
                thread::sleep(POLL_DELAY);
            }
        }
    }
}
