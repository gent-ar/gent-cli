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
        if status.is_some() {
            self.alive
                .store(false, std::sync::atomic::Ordering::Release);
        }
        Ok(status.map(|status| format!("local process exited with {status}")))
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
