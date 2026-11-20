use crate::ordinary_lifecycle_control::OrdinaryPromptPermit;
use gent_drivers::{
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    process::{ProcessOutput, SystemProcess},
    supervisor::ProviderProcess,
};

pub(crate) struct AdmittedProcess {
    process: SystemProcess,
    _permit: OrdinaryPromptPermit,
}

impl AdmittedProcess {
    pub(crate) fn new(process: SystemProcess, permit: OrdinaryPromptPermit) -> Self {
        Self {
            process,
            _permit: permit,
        }
    }
    pub(crate) fn output(&self) -> ProcessOutput {
        self.process.output()
    }
}

impl ProviderProcess for AdmittedProcess {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.process.write_frame(frame)
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.process.close_stdin()
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.process.next_stdout_chunk()
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.process.try_exit_code()
    }
}
impl ProcessTreeControl for AdmittedProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.process.signal_tree(signal)
    }
}
