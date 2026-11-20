use std::thread;
use std::time::Duration;

use crate::SystemProcess;
use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::sandboxed_launch::ProcessSandboxEvidence;
use crate::supervisor::ProviderProcess;

use super::{WindowsSandboxProcess, cleanup};

impl WindowsSandboxProcess {
    fn owned(&self) -> &SystemProcess {
        self.process.as_ref().expect("owned sandbox process")
    }
    fn unprovable() -> ProcessTreeError {
        ProcessTreeError::Failed("native sandbox scope settlement is unprovable; retained ownership requires reconciliation".into())
    }
    fn settled(&self) -> Result<bool, ProcessTreeError> {
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        let status = self.owned().peek_owned_exit_status()?;
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        match status {
            Some(status) if !matches!(status.code(), Some(0 | 125)) => Err(Self::unprovable()),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    fn stop_owned(&self, force: bool) -> Result<(), ProcessTreeError> {
        let _lock = self
            .stop_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        if self.settled()? {
            return Ok(());
        }
        if !force {
            self.owned().close_stdin()?;
            let deadline = std::time::Instant::now() + Duration::from_secs(18);
            while std::time::Instant::now() < deadline {
                if self.settled()? {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.forced
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = self.owned().kill_owned();
        let _ = self.owned().reap_owned();
        Err(Self::unprovable())
    }
}
impl ProcessTreeControl for WindowsSandboxProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.stop_owned(signal == ProcessTreeSignal::Kill)
    }
}
impl ProviderProcess for WindowsSandboxProcess {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.settled()
            .ok()
            .filter(|settled| !settled)
            .map(|_| &self.evidence)
    }
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        self.owned().write_frame(frame)
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.owned().close_stdin()
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.owned().next_stdout_chunk()
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        if self.settled()? {
            self.owned().try_exit_code()
        } else {
            Ok(None)
        }
    }
}
impl Drop for WindowsSandboxProcess {
    fn drop(&mut self) {
        if self.stop_owned(false).is_ok() {
            let output = self.owned().output();
            for line in String::from_utf8_lossy(&output.stderr.bytes).lines() {
                if line.starts_with("GENT-SANDBOX-SETTLED ") {
                    eprintln!("{line}");
                }
            }
            self.cleanup.cleanup(self.evidence.network_peer());
        } else if let (Some(process), Some(pin)) = (self.process.take(), self.helper_image.take()) {
            eprintln!(
                "native sandbox unprovable scope: {}",
                self.evidence.run_id()
            );
            cleanup::quarantine_process(process, pin);
        }
    }
}
