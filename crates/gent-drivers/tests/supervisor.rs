use std::fs;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use gent_drivers::buffering::{BufferPolicy, OfferResult, ReadDirective};
use gent_drivers::interrupt::{
    InterruptEvent, InterruptPolicy, ProcessTreeControl, ProcessTreeError, ProcessTreeSignal,
};
use gent_drivers::lock::capture;
use gent_drivers::{
    OutputLimits, ProcessLauncher, ProviderLaunch, ProviderProcess, ProviderSupervisor,
    SessionEffect, SupervisorError,
};

#[derive(Debug, Default)]
struct FakeProcess {
    signals: Arc<AtomicU8>,
}

impl ProcessTreeControl for FakeProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        let value = match signal {
            ProcessTreeSignal::Interrupt => 1,
            ProcessTreeSignal::Terminate => 2,
            ProcessTreeSignal::Kill => 3,
        };
        self.signals.store(value, Ordering::SeqCst);
        Ok(())
    }
}

impl ProviderProcess for FakeProcess {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}

#[derive(Debug)]
struct FakeLauncher {
    launches: Arc<AtomicU8>,
    signal: Arc<AtomicU8>,
}

impl ProcessLauncher for FakeLauncher {
    type Process = FakeProcess;

    fn launch(&self, _: &ProviderLaunch) -> Result<Self::Process, SupervisorError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        Ok(FakeProcess {
            signals: Arc::clone(&self.signal),
        })
    }
}

fn lock(path: &Path) -> gent_types::RunVersionLock {
    capture("claude", path, "1.0", "public").unwrap()
}

fn supervisor(path: &Path) -> ProviderSupervisor<FakeProcess> {
    ProviderSupervisor::new(
        lock(path),
        OutputLimits::new(32, 64),
        BufferPolicy::new(2, 64, 0, 0).unwrap(),
    )
}

fn launcher() -> FakeLauncher {
    FakeLauncher {
        launches: Arc::new(AtomicU8::new(0)),
        signal: Arc::new(AtomicU8::new(0)),
    }
}

#[test]
fn changed_lock_blocks_spawn_without_invoking_a_provider() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "first").unwrap();
    let mut supervisor = supervisor(&executable);
    fs::write(&executable, "second").unwrap();
    let launcher = launcher();

    assert!(matches!(
        supervisor.spawn(&launcher, vec![]),
        Err(SupervisorError::Lock(_))
    ));
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
}

#[test]
fn changed_lock_blocks_resume_without_invoking_a_provider() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "first").unwrap();
    let mut supervisor = supervisor(&executable);
    fs::write(&executable, "second").unwrap();
    let launcher = launcher();

    assert!(matches!(
        supervisor.resume(&launcher, vec!["--resume".into()], "session-a".into()),
        Err(SupervisorError::Lock(_))
    ));
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
}

#[test]
fn supervised_process_uses_bounded_frames_and_tree_interrupts() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "public").unwrap();
    let mut supervisor = supervisor(&executable);
    let launcher = launcher();
    supervisor.spawn(&launcher, vec!["--json".into()]).unwrap();

    assert_eq!(
        supervisor.offer_frame(br#"{"type":"session_started","session_id":"s"}"#.to_vec()),
        OfferResult::Queued(ReadDirective::Continue)
    );
    assert!(supervisor.drain_frame().0.is_empty());
    assert_eq!(
        supervisor.offer_frame(br#"{"type":"output","text":"ok"}"#.to_vec()),
        OfferResult::Queued(ReadDirective::Continue)
    );
    assert!(matches!(
        supervisor.drain_frame().0.as_slice(),
        [SessionEffect::Normalized { .. }]
    ));
    assert_eq!(
        supervisor
            .interrupt(
                InterruptEvent::Requested,
                InterruptPolicy {
                    interrupt_grace_ms: 7,
                    terminate_grace_ms: 9
                },
            )
            .unwrap(),
        Some(7)
    );
    assert_eq!(launcher.signal.load(Ordering::SeqCst), 1);
}

#[test]
fn private_provider_names_are_rejected_before_launch() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("private");
    fs::write(&executable, "private").unwrap();
    let mut lock = lock(&executable);
    lock.provider = "claurst".into();
    let mut supervisor = ProviderSupervisor::<FakeProcess>::new(
        lock,
        OutputLimits::new(1, 1),
        BufferPolicy::new(1, 1, 0, 0).unwrap(),
    );
    let launcher = launcher();

    assert!(matches!(
        supervisor.spawn(&launcher, vec![]),
        Err(SupervisorError::UnsupportedProvider(provider)) if provider == "claurst"
    ));
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
}
