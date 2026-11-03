use super::*;
use std::sync::Barrier;
#[test]
fn admitted_work_rechecks_policy_fence_before_native_write() {
    let (_, process, request, resources) = crate::contained_model::tests::fixture(true, false);
    let live = Arc::new(AtomicBool::new(true));
    let check = live.clone();
    let scope = Scope::new(
        process.clone(),
        request,
        "run".into(),
        "b".repeat(64),
        "unit-model".into(),
        Arc::new(move || check.load(Ordering::Acquire)),
        resources,
    );
    let admitted = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let writer = scope.clone();
    let a = admitted.clone();
    let b = release.clone();
    let task = std::thread::spawn(move || {
        assert!(writer.current());
        a.wait();
        b.wait();
        writer.write(b"must-not-reach-native-worker")
    });
    admitted.wait();
    live.store(false, Ordering::Release);
    release.wait();
    assert_eq!(task.join().unwrap(), Err(ModelError::Revoked));
    assert_eq!(process.writes.load(Ordering::Acquire), 0);
    assert!(scope.retire().is_ok() && scope.settled());
}
