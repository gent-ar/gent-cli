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
#[cfg(unix)]
use gent_drivers::{SystemLauncher, SystemProcess};

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

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
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
    assert!(matches!(
        supervisor.drain_frame().0.as_slice(),
        [SessionEffect::SessionStarted { provider_session_id }] if provider_session_id == "s"
    ));
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
fn stdout_chunks_reach_the_existing_session_reducer_in_order() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "public").unwrap();
    let mut supervisor = supervisor(&executable);
    let launcher = launcher();
    supervisor.spawn(&launcher, vec!["--json".into()]).unwrap();

    assert_eq!(
        supervisor.offer_output_chunk(br#"{"type":"session_started","session"#),
        Ok(ReadDirective::Continue)
    );
    assert_eq!(
        supervisor.offer_output_chunk(
            br#"_id":"s"}
{"type":"output","text":"ok"}
"#
        ),
        Ok(ReadDirective::Pause)
    );
    assert!(matches!(
        supervisor.drain_frame().0.as_slice(),
        [SessionEffect::SessionStarted { provider_session_id }] if provider_session_id == "s"
    ));
    assert!(matches!(
        supervisor.drain_frame().0.as_slice(),
        [SessionEffect::Normalized { .. }]
    ));
}

#[cfg(unix)]
#[test]
fn system_stdout_reaches_the_reducer_through_the_bounded_process_queue() {
    let executable = Path::new("/bin/sh");
    let mut supervisor = ProviderSupervisor::<SystemProcess>::new(
        capture("claude", executable, "1", "public").unwrap(),
        OutputLimits::new(32, 64),
        BufferPolicy::new(2, 64, 0, 0).unwrap(),
    );
    supervisor
        .spawn(
            &SystemLauncher::new(64),
            vec![
                "-c".into(),
                "printf '%s\\n' '{\"type\":\"session_started\",\"session_id\":\"s\"}' '{\"type\":\"output\",\"text\":\"ok\"}'".into(),
            ],
        )
        .unwrap();

    let mut effects = Vec::new();
    for _ in 0..100 {
        if let Some(chunk_effects) = supervisor.poll_stdout().unwrap() {
            effects.extend(chunk_effects);
        }
        if effects
            .iter()
            .any(|effect| matches!(effect, SessionEffect::Normalized { .. }))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(matches!(
        effects.as_slice(),
        [SessionEffect::SessionStarted { provider_session_id }, SessionEffect::Normalized { .. }]
            if provider_session_id == "s"
    ));
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
