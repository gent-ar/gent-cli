use std::sync::{Arc, Mutex, PoisonError, Weak};

pub type ActivityWaker = dyn Fn() + Send + Sync;

static WAKERS: Mutex<Vec<Weak<ActivityWaker>>> = Mutex::new(Vec::new());

pub fn subscribe(waker: &Arc<ActivityWaker>) {
    WAKERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::downgrade(waker));
}

pub fn signal() {
    let live: Vec<Arc<ActivityWaker>> = {
        let mut wakers = WAKERS.lock().unwrap_or_else(PoisonError::into_inner);
        wakers.retain(|waker| waker.strong_count() > 0);
        wakers.iter().filter_map(Weak::upgrade).collect()
    };
    for waker in live {
        waker();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{ActivityWaker, signal, subscribe};

    #[test]
    fn every_live_subscriber_is_woken_and_dropped_ones_are_forgotten() {
        let count = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&count);
        let waker: Arc<ActivityWaker> = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        subscribe(&waker);
        signal();
        assert!(count.load(Ordering::SeqCst) >= 1);
        drop(waker);
        let before = count.load(Ordering::SeqCst);
        signal();
        assert_eq!(count.load(Ordering::SeqCst), before);
    }
}
