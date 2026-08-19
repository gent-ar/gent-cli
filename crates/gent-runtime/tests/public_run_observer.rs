//! Regression coverage for observer authority's pre-effect lifecycle boundary.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gent_ports::{
    Ledger, PublicProviderResolver, PublicProviderRunError, PublicProviderRunner,
    RunVersionAuthorizer,
};
use gent_protocol::{
    DependencyProvider, PublicRunInterruptRequest, PublicRunOutcome, PublicRunResumeRequest,
    PublicRunStartRequest,
};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, RunVersionLock};

#[derive(Debug)]
struct Calls(Arc<AtomicUsize>);

impl Calls {
    fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }

    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl Clone for Calls {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

#[derive(Debug)]
struct CountingResolver(Calls);

impl PublicProviderResolver for CountingResolver {
    fn resolve(&self, _: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        self.0.0.fetch_add(1, Ordering::SeqCst);
        Err(PublicProviderRunError::CompatibilityDenied)
    }
}

#[derive(Debug)]
struct CountingAuthorizer(Calls);

impl RunVersionAuthorizer for CountingAuthorizer {
    fn authorize(&self, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.0.0.fetch_add(1, Ordering::SeqCst);
        Err(PublicProviderRunError::CompatibilityDenied)
    }
}

#[derive(Debug)]
struct CountingRunner {
    start: Calls,
    resume: Calls,
    interrupt: Calls,
}

impl PublicProviderRunner for CountingRunner {
    fn start(&self, _: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.start.0.fetch_add(1, Ordering::SeqCst);
        Err(PublicProviderRunError::Failed("start must not run".into()))
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        self.resume.0.fetch_add(1, Ordering::SeqCst);
        Err(PublicProviderRunError::Failed("resume must not run".into()))
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        self.interrupt.0.fetch_add(1, Ordering::SeqCst);
        Err(PublicProviderRunError::Failed(
            "interrupt must not run".into(),
        ))
    }
}

#[test]
fn observer_denies_all_public_lifecycle_requests_before_effects_or_ledger_writes() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let resolver = Calls::new();
    let authorizer = Calls::new();
    let start = Calls::new();
    let resume = Calls::new();
    let interrupt = Calls::new();
    let service = PublicRunService::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        CountingRunner {
            start: start.clone(),
            resume: resume.clone(),
            interrupt: interrupt.clone(),
        },
        CountingAuthorizer(authorizer.clone()),
        CountingResolver(resolver.clone()),
        ProviderRunAuthority::Observer,
    );

    assert_eq!(
        service.start(start_request()).unwrap().outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(
        service.resume(resume_request()).unwrap().outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(
        service.interrupt(interrupt_request()).unwrap().outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(resolver.count(), 0);
    assert_eq!(authorizer.count(), 0);
    assert_eq!(start.count(), 0);
    assert_eq!(resume.count(), 0);
    assert_eq!(interrupt.count(), 0);
    assert!(ledger.find_run("run-a").unwrap().is_none());
    assert!(ledger.find_run_version_lock("run-a").unwrap().is_none());
    assert!(ledger.find_run_session_binding("run-a").unwrap().is_none());
    assert!(ledger.find_run_lease("run-a").unwrap().is_none());
    assert!(ledger.read_event_page(0, 100).unwrap().events.is_empty());
}

fn start_request() -> PublicRunStartRequest {
    PublicRunStartRequest {
        run_id: "run-a".into(),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Claude,
        executable: "client-hint".into(),
        version: "client-hint".into(),
        compatibility_entry: "client-hint".into(),
    }
}

fn resume_request() -> PublicRunResumeRequest {
    PublicRunResumeRequest {
        run_id: "run-a".into(),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
        session_id: "client-hint".into(),
    }
}

fn interrupt_request() -> PublicRunInterruptRequest {
    PublicRunInterruptRequest {
        run_id: "run-a".into(),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
    }
}
