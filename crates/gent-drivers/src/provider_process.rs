use crate::interrupt::{ProcessTreeControl, ProcessTreeError};

pub trait ProviderProcess: ProcessTreeControl {
    fn sandbox_evidence(&self) -> Option<&crate::sandboxed_launch::ProcessSandboxEvidence> {
        None
    }
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError>;

    fn close_stdin(&self) -> Result<(), ProcessTreeError>;
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(None)
    }

    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok(None)
    }
}
