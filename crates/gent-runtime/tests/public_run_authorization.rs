use std::fs;

use gent_ports::{Ledger, PublicProviderResolver, PublicProviderRunError, PublicProviderRunner};
use gent_protocol::{DependencyProvider, PublicRunOutcome, PublicRunStartRequest};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, RunVersionLock};

mod public_run_authorizer;
mod public_run_resolver;

use public_run_authorizer::FakeAuthorizer;
use public_run_resolver::FakeResolver;

#[derive(Debug)]
struct NoStartRunner;

#[derive(Debug)]
struct DeniedResolver;

impl PublicProviderResolver for DeniedResolver {
    fn resolve(&self, _: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        Err(PublicProviderRunError::CompatibilityDenied)
    }
}

impl PublicProviderRunner for NoStartRunner {
    fn start(&self, _: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        Err(PublicProviderRunError::Failed("start must not run".into()))
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        Err(PublicProviderRunError::Failed("resume must not run".into()))
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Err(PublicProviderRunError::Failed(
            "interrupt must not run".into(),
        ))
    }
}

#[test]
fn unsigned_compatibility_never_reserves_or_starts_a_run() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "stable").unwrap();
    let lock = gent_drivers::lock::capture("claude", &executable, "observed", "entry").unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let (authorizer, _, locks) = FakeAuthorizer::new(false);
    let (resolver, _) = FakeResolver::new(lock);
    let service = PublicRunService::new(
        coordinator,
        NoStartRunner,
        authorizer,
        resolver,
        ProviderRunAuthority::PublicDrivers,
    );
    let response = service
        .start(PublicRunStartRequest {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
            provider: DependencyProvider::Claude,
            executable: "client-hint".into(),
            version: "client-hint".into(),
            compatibility_entry: "client-hint".into(),
        })
        .unwrap();

    assert_eq!(response.outcome, PublicRunOutcome::Denied);
    assert!(ledger.find_run("run-a").unwrap().is_none());
    assert_eq!(locks.lock().unwrap().len(), 1);
}

#[test]
fn unavailable_daemon_resolver_denies_before_authorization_or_process_work() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let (authorizer, _, locks) = FakeAuthorizer::new(true);
    let service = PublicRunService::new(
        coordinator,
        NoStartRunner,
        authorizer,
        DeniedResolver,
        ProviderRunAuthority::PublicDrivers,
    );
    let response = service
        .start(PublicRunStartRequest {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
            provider: DependencyProvider::Claude,
            executable: "client-hint".into(),
            version: "client-hint".into(),
            compatibility_entry: "client-hint".into(),
        })
        .unwrap();

    assert_eq!(response.outcome, PublicRunOutcome::Denied);
    assert!(ledger.find_run("run-a").unwrap().is_none());
    assert!(locks.lock().unwrap().is_empty());
}
