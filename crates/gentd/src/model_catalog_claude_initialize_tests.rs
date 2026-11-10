use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};

use gent_protocol::{ProviderAuthFrame, model_catalog::ProviderAvailability};
use gent_types::{ProviderAuthLifecycle, ProviderAuthStatus};

use super::super::{claude::ClaudeModelSource, service::ModelCatalogSource};
use super::*;
use crate::provider_auth_api::fixed::FixedProviderAuth as FixedAuth;

struct Fixture {
    _root: tempfile::TempDir,
    probes: Arc<AtomicUsize>,
    initialize: Arc<ClaudeInitialize>,
}

struct ResolvingAuth(Mutex<Option<ProviderAuthLifecycle>>);

impl ResolvingAuth {
    fn resolve(&self, lifecycle: ProviderAuthLifecycle) {
        *self.0.lock().unwrap() = Some(lifecycle);
    }
}

impl ProviderAuthPort for ResolvingAuth {
    fn exchange(&self, _: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        Err("unexpected auth frame".into())
    }

    fn check_status(&self, provider: ProviderAuthProvider) -> ProviderAuthStatus {
        FixedAuth(ProviderAuthLifecycle::Checking).check_status(provider)
    }

    fn settled_lifecycle(&self, _: ProviderAuthProvider) -> Option<ProviderAuthLifecycle> {
        *self.0.lock().unwrap()
    }
}

fn fixture(
    auth: ProviderAuthLifecycle,
    answer: Result<Value, ProviderLaunchError>,
    duration: Duration,
) -> Fixture {
    fixture_with(Arc::new(FixedAuth(auth)), answer, duration)
}

fn fixture_with(
    auth: Arc<dyn ProviderAuthPort>,
    answer: Result<Value, ProviderLaunchError>,
    duration: Duration,
) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("claude.exe");
    std::fs::write(&executable, "").unwrap();
    let probes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&probes);
    let initialize = ClaudeInitialize::with_probe(
        crate::provider_executables::ProviderExecutables::explicit(Some(executable), None),
        Some(auth),
        Box::new(move |_, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(duration);
            answer.clone()
        }),
    );
    Fixture {
        _root: root,
        probes,
        initialize,
    }
}

fn settle(initialize: &Arc<ClaudeInitialize>, workspace: Option<PathBuf>) -> InitializeState {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match initialize.state(workspace.clone(), false) {
            InitializeState::Loading if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            state => return state,
        }
    }
}

fn read_concurrently(initialize: &Arc<ClaudeInitialize>, readers: usize, reads: usize) {
    let start = Arc::new(Barrier::new(readers));
    let handles = (0..readers)
        .map(|_| {
            let (initialize, start) = (Arc::clone(initialize), Arc::clone(&start));
            std::thread::spawn(move || {
                start.wait();
                for _ in 0..reads {
                    let _ = initialize.state(None, false);
                    let _ = initialize.load(None, false);
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn repeated_catalogue_reads_within_the_ttl_cause_exactly_one_probe() {
    let fixture = fixture(
        ProviderAuthLifecycle::Authenticated,
        Ok(json!({"commands": [], "models": []})),
        Duration::from_millis(40),
    );
    read_concurrently(&fixture.initialize, 8, 25);
    assert!(matches!(
        settle(&fixture.initialize, None),
        InitializeState::Listed(_)
    ));
    read_concurrently(&fixture.initialize, 4, 25);
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_probe_is_not_retried_within_its_backoff() {
    let fixture = fixture(
        ProviderAuthLifecycle::Authenticated,
        Err(ProviderLaunchError::Failed("claude exited".into())),
        Duration::from_millis(40),
    );
    read_concurrently(&fixture.initialize, 8, 25);
    assert!(matches!(
        settle(&fixture.initialize, None),
        InitializeState::Failed(message) if message == "claude exited"
    ));
    read_concurrently(&fixture.initialize, 4, 25);
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 1);
}

#[test]
fn simultaneous_refreshes_share_one_probe() {
    let fixture = fixture(
        ProviderAuthLifecycle::Authenticated,
        Ok(json!({"models": []})),
        Duration::from_millis(200),
    );
    let start = Arc::new(Barrier::new(6));
    let handles = (0..6)
        .map(|_| {
            let (initialize, start) = (Arc::clone(&fixture.initialize), Arc::clone(&start));
            std::thread::spawn(move || {
                start.wait();
                initialize.load(None, true).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_signed_out_claude_is_never_probed_for_commands_or_models() {
    let fixture = fixture(
        ProviderAuthLifecycle::Unauthenticated,
        Ok(json!({"models": []})),
        Duration::ZERO,
    );
    read_concurrently(&fixture.initialize, 4, 25);
    assert!(matches!(
        settle(&fixture.initialize, Some(PathBuf::from("workspace"))),
        InitializeState::Failed(message) if message == "claude is not signed in"
    ));
    let listing = ClaudeModelSource {
        initialize: Arc::clone(&fixture.initialize),
        auth: Arc::new(FixedAuth(ProviderAuthLifecycle::Unauthenticated)),
    }
    .load()
    .unwrap();
    assert_eq!(listing.availability, ProviderAvailability::SignedOut);
    assert!(listing.models.is_empty());
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 0);
}

#[test]
fn an_unresolved_or_signed_out_account_launches_nothing_until_it_signs_in() {
    let auth = Arc::new(ResolvingAuth(Mutex::new(None)));
    let fixture = fixture_with(
        Arc::clone(&auth) as Arc<dyn ProviderAuthPort>,
        Ok(json!({"commands": [], "models": []})),
        Duration::from_millis(40),
    );
    read_concurrently(&fixture.initialize, 4, 25);
    assert!(matches!(
        fixture.initialize.state(None, false),
        InitializeState::Loading
    ));
    auth.resolve(ProviderAuthLifecycle::Unauthenticated);
    read_concurrently(&fixture.initialize, 4, 25);
    assert!(matches!(
        fixture.initialize.state(None, false),
        InitializeState::Failed(message) if message == SIGNED_OUT
    ));
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 0);
    auth.resolve(ProviderAuthLifecycle::Authenticated);
    assert!(matches!(
        settle(&fixture.initialize, None),
        InitializeState::Listed(_)
    ));
    read_concurrently(&fixture.initialize, 4, 25);
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 1);
}
