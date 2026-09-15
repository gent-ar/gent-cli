use gent_drivers::lock::recheck;
use gent_ports::{Ledger, PublicProviderRunError, PublicProviderRunner};
use gent_protocol::{
    DependencyProvider, PublicRunInterruptRequest, PublicRunOutcome, PublicRunResumeRequest,
    PublicRunStartRequest,
};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, RunVersionLock};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
mod public_run_authorizer;
mod public_run_resolver;
use public_run_authorizer::FakeAuthorizer;
use public_run_resolver::FakeResolver;
type TestService = PublicRunService<SqliteLedger, FakeRunner, FakeAuthorizer, FakeResolver>;
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

fn resolved_lock(path: &std::path::Path) -> RunVersionLock {
    gent_drivers::lock::capture("claude", path, "observed-version", "daemon-entry").unwrap()
}

fn service(
    authority: ProviderRunAuthority,
    runner: FakeRunner,
    lock: RunVersionLock,
) -> (TestService, AuthorizationState, Arc<AtomicUsize>) {
    let coordinator = Coordinator::new(runner.ledger.clone(), CapabilitySet::default());
    let (authorizer, allowed, locks) = FakeAuthorizer::new(true);
    let (resolver, resolver_calls) = FakeResolver::new(lock);
    (
        PublicRunService::new(coordinator, runner, authorizer, resolver, authority),
        (allowed, locks),
        resolver_calls,
    )
}

#[test]
fn observer_mode_hard_denies_provider_lifecycle_without_touching_runner() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger.clone());
    let path = tempfile::NamedTempFile::new().unwrap();
    let (service, (_, locks), resolver_calls) = service(
        ProviderRunAuthority::Observer,
        runner,
        resolved_lock(path.path()),
    );
    let answer = service.start(request(path.path())).unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::Denied);
    assert!(locks.lock().unwrap().is_empty());
    assert_eq!(resolver_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn start_persists_run_lock_and_lease_before_fake_launch() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let (service, (_, locks), resolver_calls) = service(
        ProviderRunAuthority::PublicDrivers,
        FakeRunner::new(ledger.clone()),
        resolved_lock(&executable),
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
    let persisted = ledger.find_run_version_lock("run-a").unwrap().unwrap();
    assert_eq!(persisted.version, "observed-version");
    assert_eq!(persisted.compatibility_entry, "daemon-entry");
    assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
}
#[test]
fn a_binary_changed_before_spawn_is_denied_after_durable_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut runner = FakeRunner::new(ledger.clone());
    runner.change_before_start = true;
    let (service, _, _) = service(
        ProviderRunAuthority::PublicDrivers,
        runner,
        resolved_lock(&executable),
    );
    let answer = service.start(request(&executable)).unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::Denied);
    assert_eq!(answer.run_id, "run-a");
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
    let (service, _, _) = service(
        ProviderRunAuthority::PublicDrivers,
        runner,
        resolved_lock(&executable),
    );
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
    let (service, (authorization, _), _) = service(
        ProviderRunAuthority::PublicDrivers,
        runner,
        resolved_lock(&executable),
    );
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
    let (service, _, _) = service(
        ProviderRunAuthority::PublicDrivers,
        runner,
        resolved_lock(&executable),
    );
    service.start(request(&executable)).unwrap();

    assert!(
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
            })
            .is_err()
    );
}

#[derive(Debug)]
struct CurrentFileResolver(std::path::PathBuf);

impl gent_ports::PublicProviderResolver for CurrentFileResolver {
    fn resolve(&self, _: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        Ok(resolved_lock(&self.0))
    }
}

#[test]
fn resume_rebinds_the_run_to_the_currently_authorized_executable() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "version a").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let runner = FakeRunner::new(ledger.clone());
    let resumed_sessions = Arc::clone(&runner.resumed_sessions);
    let (authorizer, allowed, _) = FakeAuthorizer::new(true);
    let service = PublicRunService::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        runner,
        authorizer,
        CurrentFileResolver(executable.clone()),
        ProviderRunAuthority::PublicDrivers,
    );
    service.start(request(&executable)).unwrap();
    service
        .record_provider_session("run-a".into(), "daemon-a", HostEpoch(1), "session".into())
        .unwrap();
    let resume = || {
        service
            .resume(PublicRunResumeRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
            })
            .unwrap()
            .outcome
    };
    assert!(service.launched_executable_is_current("run-a").unwrap());

    fs::write(&executable, "version b, upgraded").unwrap();
    assert!(!service.launched_executable_is_current("run-a").unwrap());
    assert_eq!(resume(), PublicRunOutcome::Resumed);
    assert_eq!(
        ledger.find_run_version_lock("run-a").unwrap(),
        Some(resolved_lock(&executable))
    );
    assert!(service.launched_executable_is_current("run-a").unwrap());
    assert_eq!(resumed_sessions.lock().unwrap().as_slice(), ["session"]);

    fs::write(&executable, "version c, unauthorized").unwrap();
    allowed.store(0, Ordering::SeqCst);
    assert_eq!(resume(), PublicRunOutcome::Denied);
    assert_eq!(resumed_sessions.lock().unwrap().len(), 1);
    assert_ne!(
        ledger.find_run_version_lock("run-a").unwrap(),
        Some(resolved_lock(&executable))
    );
}
