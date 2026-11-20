use std::process::{Child, ExitStatus};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::interrupt::ProcessTreeError;

type Settled = Option<Result<ExitStatus, String>>;

#[derive(Debug, Default)]
pub(crate) struct ProcessExit {
    status: Mutex<Settled>,
    settled: Condvar,
}

impl ProcessExit {
    pub(crate) fn watch(mut child: Child) -> std::io::Result<Arc<Self>> {
        let exit = Arc::new(Self::default());
        let watched = Arc::clone(&exit);
        thread::Builder::new()
            .name("gent-provider-exit".into())
            .spawn(move || {
                let status = child.wait().map_err(|error| error.to_string());
                *watched.lock() = Some(status);
                watched.settled.notify_all();
                drop(child);
                crate::provider_activity::signal();
            })?;
        Ok(exit)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Settled> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn peek(&self) -> Result<Option<ExitStatus>, ProcessTreeError> {
        settled(&self.lock())
    }

    pub(crate) fn wait(&self) -> Result<ExitStatus, ProcessTreeError> {
        let status = self
            .settled
            .wait_while(self.lock(), |status| status.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        settled(&status)?.ok_or_else(|| ProcessTreeError::Failed("provider exit was lost".into()))
    }

    #[cfg(windows)]
    pub(crate) fn wait_for(&self, limit: std::time::Duration) -> bool {
        let (status, _) = self
            .settled
            .wait_timeout_while(self.lock(), limit, |status| status.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        matches!(*status, Some(Ok(_)))
    }
}

fn settled(status: &Settled) -> Result<Option<ExitStatus>, ProcessTreeError> {
    match status {
        None => Ok(None),
        Some(Ok(status)) => Ok(Some(*status)),
        Some(Err(error)) => Err(ProcessTreeError::Failed(error.clone())),
    }
}
