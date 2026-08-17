use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gent_ports::{
    ConversationLedger, Ledger, PublicProviderResolver, PublicProviderRunError,
    PublicProviderRunner, RunVersionAuthorizer,
};
use gent_protocol::{DependencyProvider, PublicRunOutcome, PublicRunStartRequest};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, ConversationRecord, HostEpoch, RunVersionLock};

#[derive(Debug)]
struct Runner(Arc<AtomicUsize>);

impl PublicProviderRunner for Runner {
    fn start(&self, _: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        unreachable!("activation test never resumes")
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        unreachable!("activation test never interrupts")
    }
}

#[derive(Debug)]
struct Resolver;

impl PublicProviderResolver for Resolver {
    fn resolve(&self, _: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        Ok(lock())
    }
}

#[derive(Debug)]
struct Authorizer;

impl RunVersionAuthorizer for Authorizer {
    fn authorize(&self, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/verified/claude".into(),
        file_identity: "1:2".into(),
        digest_sha256: "a".repeat(64),
        version: "1.0".into(),
        compatibility_entry: "claude-1".into(),
    }
}

#[test]
fn starts_a_precreated_conversation_run_without_replacing_its_identity() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &gent_ports::RunRecord {
                run_id: "run-a".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let service = PublicRunService::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        Runner(Arc::clone(&starts)),
        Authorizer,
        Resolver,
        ProviderRunAuthority::PublicDrivers,
    );

    let response = service
        .start(PublicRunStartRequest {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
            provider: DependencyProvider::Claude,
            executable: "client-supplied-and-ignored".into(),
            version: "client-supplied-and-ignored".into(),
            compatibility_entry: "client-supplied-and-ignored".into(),
        })
        .unwrap();

    assert_eq!(response.outcome, PublicRunOutcome::Started);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        ledger.find_run_version_lock("run-a").unwrap().unwrap(),
        lock()
    );
    assert_eq!(
        ledger
            .find_run_lease("run-a")
            .unwrap()
            .unwrap()
            .coordinator_id,
        "daemon-a"
    );
    assert_eq!(
        ledger
            .find_conversation("conversation-a")
            .unwrap()
            .unwrap()
            .conversation_id,
        "conversation-a"
    );
}

#[test]
fn refuses_to_retarget_an_existing_conversation_run_to_another_provider() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &gent_ports::RunRecord {
                run_id: "run-a".into(),
                parent_run_id: None,
                provider: "codex".into(),
            },
        )
        .unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let service = PublicRunService::new(
        Coordinator::new(ledger, CapabilitySet::default()),
        Runner(Arc::clone(&starts)),
        Authorizer,
        Resolver,
        ProviderRunAuthority::PublicDrivers,
    );

    assert!(
        service
            .start(PublicRunStartRequest {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
                provider: DependencyProvider::Claude,
                executable: String::new(),
                version: String::new(),
                compatibility_entry: String::new(),
            })
            .is_err()
    );
    assert_eq!(starts.load(Ordering::SeqCst), 0);
}
