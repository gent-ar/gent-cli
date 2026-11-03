use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::{ProcessSandboxEvidence, ProviderProcess};

pub(crate) struct RoutedProviderProcess {
    process: Box<dyn ProviderProcess + Send>,
}

impl std::fmt::Debug for RoutedProviderProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoutedProviderProcess")
            .finish_non_exhaustive()
    }
}

impl RoutedProviderProcess {
    pub(crate) fn new(process: impl ProviderProcess + Send + 'static) -> Self {
        Self {
            process: Box::new(process),
        }
    }
}

impl ProcessTreeControl for RoutedProviderProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.process.signal_tree(signal)
    }
}

impl ProviderProcess for RoutedProviderProcess {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.process.sandbox_evidence()
    }
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
