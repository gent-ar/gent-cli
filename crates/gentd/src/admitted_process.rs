use crate::ordinary_lifecycle_control::OrdinaryPromptPermit;
use gent_drivers::{
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    process::{ProcessOutput, SystemProcess},
    supervisor::ProviderProcess,
};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

pub(crate) struct AdmittedProcess {
    process: Option<SystemProcess>,
    permit: Option<OrdinaryPromptPermit>,
}

impl AdmittedProcess {
    pub(crate) fn new(process: SystemProcess, permit: OrdinaryPromptPermit) -> Self {
        Self {
            process: Some(process),
            permit: Some(permit),
        }
    }
    fn inner(&self) -> &SystemProcess {
        self.process
            .as_ref()
            .expect("admitted child exists until drop")
    }
    pub(crate) fn output(&self) -> ProcessOutput {
        self.inner().output()
    }
}

impl ProviderProcess for AdmittedProcess {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.inner().write_frame(frame)
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.inner().close_stdin()
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.inner().next_stdout_chunk()
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.inner().try_exit_code()
    }
}
impl ProcessTreeControl for AdmittedProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.inner().signal_tree(signal)
    }
}

impl Drop for AdmittedProcess {
    fn drop(&mut self) {
        let Some(process) = self.process.take() else {
            return;
        };
        let permit = self.permit.take();
        if matches!(process.try_exit_code(), Ok(Some(_))) {
            return;
        }
        let _ = process.close_stdin();
        let _ = process.signal_tree(ProcessTreeSignal::Kill);
        let pending = Arc::new(Mutex::new(Some((process, permit))));
        let child = Arc::clone(&pending);
        let spawned = thread::Builder::new()
            .name("gent-child-reaper".into())
            .spawn(move || {
                let owned = child
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let Some((process, _permit)) = owned else {
                    return;
                };
                loop {
                    if matches!(process.try_exit_code(), Ok(Some(_))) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
        if spawned.is_err() {
            std::mem::forget(pending);
        }
    }
}
