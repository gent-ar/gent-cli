use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use gent_drivers::lock::recheck;
use gent_ports::{Ledger, PublicProviderRunError, PublicProviderRunner};
use gent_protocol::{
    DependencyProvider, PublicRunInterruptRequest, PublicRunOutcome, PublicRunResumeRequest,
    PublicRunStartRequest,
};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, RunVersionLock};

#[derive(Debug)]
struct FakeRunner {
    ledger: SqliteLedger,
    starts: AtomicUsize,
    resumes: AtomicUsize,
    interrupts: AtomicUsize,
    change_before_start: bool,
}

impl FakeRunner {
    fn new(ledger: SqliteLedger) -> Self {
        Self {
            ledger,
            starts: AtomicUsize::new(0),
            resumes: AtomicUsize::new(0),
            interrupts: AtomicUsize::new(0),
            change_before_start: false,
        }
    }
}

impl PublicProviderRunner for FakeRunner {
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        assert!(self.ledger.find_run(run_id).unwrap().is_some());
        assert_eq!(
            self.ledger.find_run_version_lock(run_id).unwrap().as_ref(),
            Some(lock)
        );
        assert!(self.ledger.find_run_lease(run_id).unwrap().is_some());
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.change_before_start {
            fs::write(&lock.canonical_path, "changed").unwrap();
        }
        recheck(lock).map_err(|_| PublicProviderRunError::ProviderChanged)
    }

    fn resume(
        &self,
        _: &str,
        lock: &RunVersionLock,
        _: &str,
    ) -> Result<(), PublicProviderRunError> {
        self.resumes.fetch_add(1, Ordering::SeqCst);
        recheck(lock).map_err(|_| PublicProviderRunError::ProviderChanged)
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        self.interrupts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn request(path: &std::path::Path) -> PublicRunStartRequest {
    PublicRunStartRequest {
        run_id: "run-a".into(),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Claude,
        executable: path.display().to_string(),
        version: "1".into(),
        compatibility_entry: "fixture".into(),
    }
}

fn service(
    authority: ProviderRunAuthority,
    runner: FakeRunner,
) -> PublicRunService<SqliteLedger, FakeRunner> {
    let coordinator = Coordinator::new(runner.ledger.clone(), CapabilitySet::default());
    PublicRunService::new(coordinator, runner, authority)
}

#[test]
fn observer_mode_hard_denies_provider_lifecycle_without_touching_runner() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger);
    let path = tempfile::NamedTempFile::new().unwrap();
    let answer = service(ProviderRunAuthority::Observer, runner)
        .start(request(path.path()))
        .unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::Denied);
}

#[test]
fn start_persists_run_lock_and_lease_before_fake_launch() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let answer = service(
        ProviderRunAuthority::PublicDrivers,
        FakeRunner::new(ledger.clone()),
    )
    .start(request(&executable))
    .unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::Started);
    assert!(ledger.find_run("run-a").unwrap().is_some());
    assert!(ledger.find_run_version_lock("run-a").unwrap().is_some());
    assert!(ledger.find_run_lease("run-a").unwrap().is_some());
}

#[test]
fn changed_binary_returns_provider_changed_after_durable_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut runner = FakeRunner::new(ledger.clone());
    runner.change_before_start = true;
    let answer = service(ProviderRunAuthority::PublicDrivers, runner)
        .start(request(&executable))
        .unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::ProviderChanged);
    assert!(ledger.find_run_version_lock("run-a").unwrap().is_some());
}

#[test]
fn resume_and_interrupt_require_authoritative_owned_run() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger);
    let service = service(ProviderRunAuthority::PublicDrivers, runner);
    assert_eq!(
        service.start(request(&executable)).unwrap().outcome,
        PublicRunOutcome::Started
    );
    assert_eq!(
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "other".into(),
                host_epoch: HostEpoch(1),
                session_id: "session".into(),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::LeaseContended
    );
    assert_eq!(
        service
            .interrupt(PublicRunInterruptRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Interrupted
    );
}
