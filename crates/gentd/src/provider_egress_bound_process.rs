use std::sync::{Arc, Mutex, OnceLock};

use gent_drivers::ProviderProcess;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};

use super::composition::ProviderGatewayLease;

fn retained_leases() -> &'static Mutex<Vec<Arc<ProviderGatewayLease>>> {
    static RETAINED: OnceLock<Mutex<Vec<Arc<ProviderGatewayLease>>>> = OnceLock::new();
    RETAINED.get_or_init(|| Mutex::new(Vec::new()))
}

pub(super) fn retain_unsettled(lease: &Arc<ProviderGatewayLease>) {
    lease.revoke();
    let mut retained = retained_leases()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !retained.iter().any(|held| Arc::ptr_eq(held, lease)) {
        retained.push(lease.clone());
    }
}

#[cfg(all(test, windows))]
pub(super) fn release_test_retention(lease: &Arc<ProviderGatewayLease>) {
    retained_leases()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|held| !Arc::ptr_eq(held, lease));
}

pub struct GatewayBoundProcess<P: ProviderProcess> {
    pub(super) process: Arc<P>,
    pub(super) lease: Arc<ProviderGatewayLease>,
}

impl<P: ProviderProcess> std::fmt::Debug for GatewayBoundProcess<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayBoundProcess")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

impl<P: ProviderProcess> GatewayBoundProcess<P> {
    fn binding_current(&self) -> Result<(), ProcessTreeError> {
        if self.lease.fence.current() {
            return Ok(());
        }
        let _ = self.process.signal_tree(ProcessTreeSignal::Terminate);
        self.lease.revoke();
        Err(ProcessTreeError::Failed(
            "provider gateway run binding changed".into(),
        ))
    }

    fn current(&self) -> Result<(), ProcessTreeError> {
        if self.lease.current() {
            return Ok(());
        }
        let _ = self.process.signal_tree(ProcessTreeSignal::Terminate);
        self.lease.revoke();
        Err(ProcessTreeError::Failed(
            "provider gateway run binding changed".into(),
        ))
    }
}

impl<P: ProviderProcess> ProcessTreeControl for GatewayBoundProcess<P> {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        if matches!(
            signal,
            ProcessTreeSignal::Terminate | ProcessTreeSignal::Kill
        ) {
            self.lease.revoke();
        }
        self.process.signal_tree(signal)
    }
}

impl<P: ProviderProcess> ProviderProcess for GatewayBoundProcess<P> {
    fn sandbox_evidence(&self) -> Option<&gent_drivers::sandboxed_launch::ProcessSandboxEvidence> {
        self.current().ok()?;
        if !matches!(self.process.try_exit_code(), Ok(None)) {
            self.lease.revoke();
            return None;
        }
        self.process.sandbox_evidence()
    }

    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.current()?;
        #[cfg(any(windows, target_os = "linux"))]
        self.lease.observe_mcp(frame, false)?;
        self.process.write_frame(frame)
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.process.close_stdin()
    }

    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.binding_current()?;
        let chunk = self.process.next_stdout_chunk()?;
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(bytes) = &chunk {
            self.lease.observe_mcp(bytes, true)?;
        }
        Ok(chunk)
    }

    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.binding_current()?;
        let result = self.process.try_exit_code();
        if !matches!(result, Ok(None)) {
            self.lease.revoke();
        }
        if matches!(result, Ok(Some(_))) && !self.lease.mcp_settled() {
            return Ok(None);
        }
        result
    }
}

impl<P: ProviderProcess> Drop for GatewayBoundProcess<P> {
    fn drop(&mut self) {
        let stopped = self
            .process
            .signal_tree(ProcessTreeSignal::Terminate)
            .is_ok();
        self.lease.revoke();
        if !stopped
            || !matches!(self.process.try_exit_code(), Ok(Some(_)))
            || !self.lease.mcp_settled()
        {
            retain_unsettled(&self.lease);
        }
    }
}
