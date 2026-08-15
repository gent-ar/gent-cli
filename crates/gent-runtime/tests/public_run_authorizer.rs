use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gent_ports::{PublicProviderRunError, RunVersionAuthorizer};
use gent_types::RunVersionLock;

#[derive(Debug)]
pub struct FakeAuthorizer {
    allowed: Arc<AtomicUsize>,
    locks: Arc<Mutex<Vec<RunVersionLock>>>,
}

impl FakeAuthorizer {
    pub fn new(allowed: bool) -> (Self, Arc<AtomicUsize>, Arc<Mutex<Vec<RunVersionLock>>>) {
        let allowed_flag = Arc::new(AtomicUsize::new(usize::from(allowed)));
        let locks = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                allowed: Arc::clone(&allowed_flag),
                locks: Arc::clone(&locks),
            },
            allowed_flag,
            locks,
        )
    }
}

impl RunVersionAuthorizer for FakeAuthorizer {
    fn authorize(&self, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.locks.lock().unwrap().push(lock.clone());
        (self.allowed.load(Ordering::SeqCst) == 1)
            .then_some(())
            .ok_or(PublicProviderRunError::CompatibilityDenied)
    }
}
