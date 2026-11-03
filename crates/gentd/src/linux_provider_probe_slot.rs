//! Scope-authority storage only. The caller must prove THAT scope Known before
//! release; this does not inspect processes or manufacture containment evidence.
use std::sync::{Arc, Mutex};
pub(super) struct Slot<T> {
    current: Mutex<Option<Arc<T>>>,
}
impl<T> Slot<T> {
    pub(super) const fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }
    pub(super) fn busy(&self) -> bool {
        self.current.lock().map_or(true, |v| v.is_some())
    }
    pub(super) fn reserve(&self, value: Arc<T>) -> Result<(), &'static str> {
        let mut slot = self
            .current
            .lock()
            .map_err(|_| "Linux preflight ownership poisoned")?;
        if slot.is_some() {
            return Err("Linux preflight ownership is active or unresolved");
        }
        *slot = Some(value);
        Ok(())
    }
    pub(super) fn release_after_scope_known(&self, value: &Arc<T>) -> bool {
        let Ok(mut slot) = self.current.lock() else {
            return false;
        };
        if !slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, value)) {
            return false;
        }
        *slot = None;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Authority(Arc<AtomicUsize>);
    impl Drop for Authority {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn unknown_return_keeps_exact_authority_and_blocks_another_attempt() {
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = Slot::new();
        let first = Arc::new(Authority(drops.clone()));
        slot.reserve(first.clone()).unwrap();
        drop(first);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(slot.busy());
        let second = Arc::new(Authority(drops.clone()));
        assert!(slot.reserve(second.clone()).is_err());
        assert!(!slot.release_after_scope_known(&second));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn only_same_authority_can_release_after_known_settlement() {
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = Slot::new();
        let first = Arc::new(Authority(drops.clone()));
        slot.reserve(first.clone()).unwrap();
        let wrong = Arc::new(Authority(drops.clone()));
        assert!(!slot.release_after_scope_known(&wrong));
        assert!(slot.busy());
        assert!(slot.release_after_scope_known(&first));
        drop(first);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(!slot.busy());
    }
}
