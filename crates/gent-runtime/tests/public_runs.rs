use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gent_drivers::lock::recheck;
use gent_ports::{Ledger, PublicProviderRunError, PublicProviderRunner};
use gent_protocol::{
    DependencyProvider, PublicRunInterruptRequest, PublicRunOutcome, PublicRunResumeRequest,
    PublicRunStartRequest,
};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, RunVersionLock};

mod public_run_authorizer;

use public_run_authorizer::FakeAuthorizer;

type TestService = PublicRunService<SqliteLedger, FakeRunner, FakeAuthorizer>;
type AuthorizationState = (Arc<AtomicUsize>, Arc<Mutex<Vec<RunVersionLock>>>);

#[derive(Debug)]
struct FakeRunner {
    ledger: SqliteLedger,
    starts: AtomicUsize,
    resumes: AtomicUsize,
    interrupts: AtomicUsize,
    change_before_start: bool,
    resumed_sessions: Arc<Mutex<Vec<String>>>,
}

impl FakeRunner {
    fn new(ledger: SqliteLedger) -> Self {
        Self {
            ledger,
            starts: AtomicUsize::new(0),
            resumes: AtomicUsize::new(0),
            interrupts: AtomicUsize::new(0),
            change_before_start: false,
            resumed_sessions: Arc::new(Mutex::new(Vec::new())),
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
        session_id: &str,
    ) -> Result<(), PublicProviderRunError> {
        self.resumes.fetch_add(1, Ordering::SeqCst);
        self.resumed_sessions
            .lock()
            .unwrap()
            .push(session_id.into());
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
) -> (TestService, AuthorizationState) {
    let coordinator = Coordinator::new(runner.ledger.clone(), CapabilitySet::default());
    let (authorizer, allowed, locks) = FakeAuthorizer::new(true);
    (
        PublicRunService::new(coordinator, runner, authorizer, authority),
        (allowed, locks),
    )
}

#[test]
fn observer_mode_hard_denies_provider_lifecycle_without_touching_runner() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger.clone());
    let path = tempfile::NamedTempFile::new().unwrap();
    let (service, (_, locks)) = service(ProviderRunAuthority::Observer, runner);
    let answer = service.start(request(path.path())).unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::Denied);
    assert!(locks.lock().unwrap().is_empty());
}

#[test]
fn start_persists_run_lock_and_lease_before_fake_launch() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let (service, (_, locks)) = service(
        ProviderRunAuthority::PublicDrivers,
        FakeRunner::new(ledger.clone()),
    );
    let answer = service.start(request(&executable)).unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::Started);
    assert!(ledger.find_run("run-a").unwrap().is_some());
    assert!(ledger.find_run_version_lock("run-a").unwrap().is_some());
    assert!(ledger.find_run_lease("run-a").unwrap().is_some());
    assert_eq!(
        locks.lock().unwrap()[0].canonical_path,
        executable.canonicalize().unwrap().display().to_string()
    );
}

#[test]
fn changed_binary_returns_provider_changed_after_durable_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut runner = FakeRunner::new(ledger.clone());
    runner.change_before_start = true;
    let (service, _) = service(ProviderRunAuthority::PublicDrivers, runner);
    let answer = service.start(request(&executable)).unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::ProviderChanged);
    assert!(ledger.find_run_version_lock("run-a").unwrap().is_some());
}

#[test]
fn resume_and_interrupt_require_authoritative_owned_run() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger.clone());
    let resumed_sessions = Arc::clone(&runner.resumed_sessions);
    let (service, _) = service(ProviderRunAuthority::PublicDrivers, runner);
    assert_eq!(
        service.start(request(&executable)).unwrap().outcome,
        PublicRunOutcome::Started
    );
    service
        .record_provider_session(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            "durable-session".into(),
        )
        .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    assert_eq!(
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-b".into(),
                host_epoch: HostEpoch(2),
                session_id: "client-substitution-attempt".into(),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Resumed
    );
    assert_eq!(
        resumed_sessions.lock().unwrap().as_slice(),
        ["durable-session"]
    );
    assert_eq!(
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "other".into(),
                host_epoch: HostEpoch(2),
                session_id: "client-substitution-attempt".into(),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::LeaseContended
    );
    assert_eq!(
        service
            .interrupt(PublicRunInterruptRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-b".into(),
                host_epoch: HostEpoch(2),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Interrupted
    );
}

#[test]
fn resume_reauthorizes_before_reclaiming_the_provider_process() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger.clone());
    let resumed_sessions = Arc::clone(&runner.resumed_sessions);
    let (service, (authorization, _)) = service(ProviderRunAuthority::PublicDrivers, runner);
    service.start(request(&executable)).unwrap();
    service
        .record_provider_session(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            "durable-session".into(),
        )
        .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    authorization.store(0, Ordering::SeqCst);

    assert_eq!(
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-b".into(),
                host_epoch: HostEpoch(2),
                session_id: String::new(),
            })
            .unwrap()
            .outcome,
        PublicRunOutcome::Denied
    );
    assert!(resumed_sessions.lock().unwrap().is_empty());
}

#[test]
fn resume_refuses_runs_without_a_server_owned_session() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger);
    let (service, _) = service(ProviderRunAuthority::PublicDrivers, runner);
    service.start(request(&executable)).unwrap();

    assert!(
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
                session_id: "client-substitution-attempt".into(),
            })
            .is_err()
    );
}

#[test]
fn unsigned_compatibility_never_reserves_or_starts_a_run() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger.clone());
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let (authorizer, _, locks) = FakeAuthorizer::new(false);
    let service = PublicRunService::new(
        coordinator,
        runner,
        authorizer,
        ProviderRunAuthority::PublicDrivers,
    );

    assert_eq!(
        service.start(request(&executable)).unwrap().outcome,
        PublicRunOutcome::Denied
    );
    assert!(ledger.find_run("run-a").unwrap().is_none());
    assert_eq!(locks.lock().unwrap().len(), 1);
}
