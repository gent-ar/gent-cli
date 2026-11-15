use super::{Arc, LocalRuntimeProcess, SystemLocalRuntimeProcess, shutdown_process_tree};

impl SystemLocalRuntimeProcess {
    pub(crate) fn liveness(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.alive.clone()
    }
    #[must_use]
    pub(crate) fn process_id(&self) -> u32 {
        self.child.id()
    }
}

impl LocalRuntimeProcess for SystemLocalRuntimeProcess {
    fn exited(&mut self) -> Result<Option<String>, String> {
        let status = self.child.try_wait().map_err(|error| error.to_string())?;
        if let Some(status) = status {
            self.alive
                .store(false, std::sync::atomic::Ordering::Release);
            let detail = self
                .startup
                .failure()
                .map_or_else(String::new, |reason| format!(". {reason}"));
            return Ok(Some(format!("local process exited with {status}{detail}")));
        }
        Ok(None)
    }

    fn wait_for_port(&mut self) -> Result<Option<u16>, String> {
        use std::time::{Duration, Instant};
        let timeout = crate::provider_launch_budget::launch_budget(Duration::from_secs(300));
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(error) = self.exited()? {
                return Err(error);
            }
            if let Some(port) = self.startup.port() {
                return Ok(Some(port));
            }
            if Instant::now() >= deadline {
                return Err("the local model did not report its listening port".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn shutdown(&mut self) -> Result<(), String> {
        self.alive
            .store(false, std::sync::atomic::Ordering::Release);
        if self.tree_stopped {
            return Ok(());
        }
        shutdown_process_tree(&mut self.child)?;
        self.tree_stopped = true;
        Ok(())
    }
}

impl Drop for SystemLocalRuntimeProcess {
    fn drop(&mut self) {
        self.alive
            .store(false, std::sync::atomic::Ordering::Release);
        if !self.tree_stopped {
            let _ = shutdown_process_tree(&mut self.child);
        }
    }
}
