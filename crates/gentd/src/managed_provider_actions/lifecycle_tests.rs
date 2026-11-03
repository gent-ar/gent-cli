use super::*;
#[test]
fn inflight_update_holds_lifecycle_admission_until_settled() {
    use crate::ordinary_lifecycle_control::{OrdinaryLifecycleControl, OrdinaryLifecyclePhase};
    let (b, e, _) = fixture();
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    let (entered, started) = std::sync::mpsc::sync_channel(0);
    let (resume, continued) = std::sync::mpsc::channel();
    *e.barrier.lock().unwrap() = Some((entered, continued));
    let req = request(&b, "lifecycle");
    let worker_control = control.clone();
    let worker =
        std::thread::spawn(move || super::with_operation(&worker_control, || Some(b.execute(req))));
    started
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(
        control
            .quiesce_if_idle(|| panic!("inflight update must not close ingress"))
            .is_err()
    );
    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Ready);
    resume.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap().unwrap().state,
        DependencyActionState::Completed
    );
    control.quiesce_if_idle(|| Ok(())).unwrap();
    assert!(
        super::with_operation(&control, || panic!(
            "closed admission must not invoke update"
        ))
        .unwrap()
        .is_err()
    );
}
