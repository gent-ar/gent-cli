use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gent_ports::{
    Ledger, PublicProviderResolver, PublicProviderRunError, PublicProviderRunner,
    RunVersionAuthorizer,
};
use gent_protocol::{DependencyProvider, PublicRunOutcome, PublicRunStartRequest};
use gent_runtime::{Coordinator, ProviderRunAuthority, PublicRunService};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, RunVersionLock};

type Starts = Arc<Mutex<Vec<(String, RunVersionLock)>>>;

#[derive(Debug)]
struct SequenceResolver {
    locks: Mutex<VecDeque<RunVersionLock>>,
    calls: Arc<AtomicUsize>,
}

impl SequenceResolver {
    fn new(locks: impl IntoIterator<Item = RunVersionLock>) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                locks: Mutex::new(locks.into_iter().collect()),
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl PublicProviderResolver for SequenceResolver {
    fn resolve(&self, _: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.locks
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(PublicProviderRunError::CompatibilityDenied)
    }
}

#[derive(Debug)]
struct RecordingRunner {
    changed_starts: AtomicUsize,
    starts: Starts,
}

impl RecordingRunner {
    fn new(changed_starts: usize) -> (Self, Starts) {
        let starts = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                changed_starts: AtomicUsize::new(changed_starts),
                starts: Arc::clone(&starts),
            },
            starts,
        )
    }
}

impl PublicProviderRunner for RecordingRunner {
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.starts
            .lock()
            .unwrap()
            .push((run_id.into(), lock.clone()));
        let changed =
            self.changed_starts
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                    count.checked_sub(1)
                });
        if changed.is_ok() {
            Err(PublicProviderRunError::ProviderChanged)
        } else {
            Ok(())
        }
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        unreachable!("this guard is evaluated before any resume launch")
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        unreachable!("this guard does not start a process")
    }
}

#[derive(Debug)]
struct AllowAll;

impl RunVersionAuthorizer for AllowAll {
    fn authorize(&self, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

fn lock(provider: &str, digest: &str) -> RunVersionLock {
    RunVersionLock {
        provider: provider.into(),
        canonical_path: format!("/fixture/{provider}"),
        file_identity: format!("fixture-{digest}"),
        digest_sha256: digest.into(),
        version: "fixture".into(),
        compatibility_entry: "fixture".into(),
    }
}

fn request() -> PublicRunStartRequest {
    PublicRunStartRequest {
        run_id: "root".into(),
        coordinator_id: "daemon".into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Claude,
        executable: "/ignored/client-value".into(),
        version: "ignored".into(),
        compatibility_entry: "ignored".into(),
    }
}

#[test]
fn resolver_cannot_silently_substitute_a_different_provider() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (resolver, calls) = SequenceResolver::new([lock("codex", "wrong-provider")]);
    let (runner, starts) = RecordingRunner::new(0);
    let service = PublicRunService::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        runner,
        AllowAll,
        resolver,
        ProviderRunAuthority::PublicDrivers,
    );

    assert_eq!(
        service.start(request()).unwrap().outcome,
        PublicRunOutcome::Denied
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(starts.lock().unwrap().is_empty());
    assert!(ledger.find_run("root").unwrap().is_none());
}

#[test]
fn changed_binary_child_uses_a_fresh_matching_lock_without_mutating_parent() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let old = lock("claude", "old");
    let fresh = lock("claude", "fresh");
    let (resolver, calls) = SequenceResolver::new([old.clone(), fresh.clone()]);
    let (runner, starts) = RecordingRunner::new(1);
    let service = PublicRunService::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        runner,
        AllowAll,
        resolver,
        ProviderRunAuthority::PublicDrivers,
    );

    let answer = service.start(request()).unwrap();
    assert_eq!(answer.outcome, PublicRunOutcome::ProviderChanged);
    assert_ne!(answer.run_id, "root");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(ledger.find_run_version_lock("root").unwrap(), Some(old));
    assert_eq!(
        ledger.find_run_version_lock(&answer.run_id).unwrap(),
        Some(fresh)
    );
    assert_eq!(
        ledger
            .find_run(&answer.run_id)
            .unwrap()
            .unwrap()
            .parent_run_id,
        Some("root".into())
    );
    assert_eq!(starts.lock().unwrap().len(), 2);
}
