use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

#[derive(Debug)]
pub struct FakeResolver {
    lock: RunVersionLock,
    calls: Arc<AtomicUsize>,
}

impl FakeResolver {
    pub fn new(lock: RunVersionLock) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                lock,
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl PublicProviderResolver for FakeResolver {
    fn resolve(&self, _: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.lock.clone())
    }
}
