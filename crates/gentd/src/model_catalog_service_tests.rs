use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use gent_protocol::model_catalog::{
    CatalogModel, ModelCatalogSelection, ModelListing, ProviderAvailability,
};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider};

use super::{ModelCatalogService, ModelCatalogSource, ProviderListing};
use crate::{
    provider_launch_budget::{ProbeRetry, ProviderLaunchError},
    runtime_facade::model_catalog::defaults::ModelCatalogDefaults,
};

struct GatedSource {
    provider: AgentChatProvider,
    gate: std::sync::Mutex<mpsc::Receiver<Result<ProviderListing, ProviderLaunchError>>>,
    loads: AtomicUsize,
}

impl ModelCatalogSource for GatedSource {
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }

    fn label(&self) -> &'static str {
        "Provider"
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(600)
    }

    fn load(&self) -> Result<ProviderListing, ProviderLaunchError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.gate
            .lock()
            .unwrap()
            .recv()
            .unwrap_or_else(|_| Err(ProviderLaunchError::Failed("closed".into())))
    }
}

fn gated(
    provider: AgentChatProvider,
) -> (
    Arc<GatedSource>,
    mpsc::Sender<Result<ProviderListing, ProviderLaunchError>>,
) {
    let (send, receive) = mpsc::channel();
    (
        Arc::new(GatedSource {
            provider,
            gate: std::sync::Mutex::new(receive),
            loads: AtomicUsize::new(0),
        }),
        send,
    )
}

fn model(id: &str, default_effort: Option<AgentChatEffort>) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        label: id.into(),
        description: None,
        is_default: false,
        efforts: default_effort.into_iter().collect(),
        default_effort,
        local: None,
    }
}

fn eventually(
    service: &ModelCatalogService,
    ready: impl Fn(&gent_protocol::model_catalog::ModelCatalog) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready(&service.snapshot()) {
        assert!(Instant::now() < deadline, "catalog did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_slow_provider_never_blocks_the_catalog_or_the_other_providers() {
    let directory = tempfile::tempdir().unwrap();
    let (local, local_gate) = gated(AgentChatProvider::Claurst);
    let (claude, claude_gate) = gated(AgentChatProvider::Claude);
    let (codex, codex_gate) = gated(AgentChatProvider::Codex);
    let service = ModelCatalogService::new(
        vec![local, claude, codex],
        ModelCatalogDefaults::open(directory.path()),
    );

    let started = Instant::now();
    let first = service.read(false);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(
        first
            .providers
            .iter()
            .map(|entry| entry.provider)
            .collect::<Vec<_>>(),
        [
            AgentChatProvider::Claurst,
            AgentChatProvider::Claude,
            AgentChatProvider::Codex
        ]
    );
    assert!(
        first
            .providers
            .iter()
            .all(|entry| entry.listing == ModelListing::Loading)
    );

    local_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![model("qwen3-1-7b-q4-k-m", Some(AgentChatEffort::Medium))],
        }))
        .unwrap();
    codex_gate
        .send(Err(ProviderLaunchError::Failed("codex exited".into())))
        .unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
            && matches!(catalog.providers[2].listing, ModelListing::Failed { .. })
    });
    let catalog = service.read(false);
    assert_eq!(catalog.providers[1].listing, ModelListing::Loading);
    assert_eq!(
        catalog.providers[1].availability,
        ProviderAvailability::Checking
    );
    assert_eq!(catalog.providers[0].models.len(), 1);
    drop(claude_gate);
}

#[test]
fn refresh_reloads_a_cached_provider_but_a_plain_read_honors_its_ttl() {
    let directory = tempfile::tempdir().unwrap();
    let (codex, codex_gate) = gated(AgentChatProvider::Codex);
    let service = ModelCatalogService::new(
        vec![Arc::clone(&codex) as Arc<dyn ModelCatalogSource>],
        ModelCatalogDefaults::open(directory.path()),
    );
    service.read(false);
    codex_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::SignedOut,
            models: vec![model("gpt-6-astra", Some(AgentChatEffort::Medium))],
        }))
        .unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    service.read(false);
    assert_eq!(codex.loads.load(Ordering::SeqCst), 1);

    let refreshing = service.read(true);
    assert_eq!(refreshing.providers[0].listing, ModelListing::Loading);
    assert_eq!(refreshing.providers[0].models[0].id, "gpt-6-astra");
    codex_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![model("gpt-7", None)],
        }))
        .unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    let refreshed = service.snapshot();
    assert_eq!(codex.loads.load(Ordering::SeqCst), 2);
    assert_eq!(refreshed.providers[0].models[0].id, "gpt-7");
    assert_eq!(
        refreshed.providers[0].availability,
        ProviderAvailability::Ready
    );
}

#[test]
fn defaults_to_the_small_local_model_until_a_listed_default_is_saved() {
    let directory = tempfile::tempdir().unwrap();
    let (local, local_gate) = gated(AgentChatProvider::Claurst);
    let (codex, codex_gate) = gated(AgentChatProvider::Codex);
    let service = ModelCatalogService::new(
        vec![local, codex],
        ModelCatalogDefaults::open(directory.path()),
    );
    let built_in = service.default_agent_selection();
    assert_eq!(built_in.provider, AgentChatProvider::Claurst);
    assert_eq!(built_in.model, "qwen3-1-7b-q4-k-m");
    assert_eq!(built_in.mode, AgentChatMode::Agent);

    service.read(false);
    local_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![model("qwen3-1-7b-q4-k-m", Some(AgentChatEffort::Medium))],
        }))
        .unwrap();
    codex_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![model("gpt-6-astra", Some(AgentChatEffort::High))],
        }))
        .unwrap();
    eventually(&service, |catalog| {
        catalog
            .providers
            .iter()
            .all(|entry| entry.listing == ModelListing::Ready)
    });
    assert!(
        service
            .set_default(ModelCatalogSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-unlisted".into(),
            })
            .is_err()
    );
    let saved = service
        .set_default(ModelCatalogSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-6-astra".into(),
        })
        .unwrap();
    assert_eq!(saved.default_selection.model, "gpt-6-astra");
    let chosen = service.default_agent_selection();
    assert_eq!(chosen.provider, AgentChatProvider::Codex);
    assert_eq!(chosen.effort, AgentChatEffort::High);
    assert_eq!(
        ModelCatalogDefaults::open(directory.path()).selection(),
        saved.default_selection
    );
}

#[path = "model_catalog_validation_tests.rs"]
mod validation;

fn listing(id: &str) -> ProviderListing {
    ProviderListing {
        availability: ProviderAvailability::Ready,
        models: vec![model(id, None)],
    }
}

fn loads_reach(source: &GatedSource, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while source.loads.load(Ordering::SeqCst) < count {
        assert!(Instant::now() < deadline, "provider was not reloaded");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_timed_out_provider_probe_keeps_checking_and_is_retried_until_it_lists() {
    let directory = tempfile::tempdir().unwrap();
    let (claude, gate) = gated(AgentChatProvider::Claude);
    let service = ModelCatalogService::new(
        vec![claude.clone()],
        ModelCatalogDefaults::open(directory.path()),
    )
    .with_retry(ProbeRetry {
        attempts: 3,
        delay: Duration::from_millis(1),
    });

    service.read(false);
    for attempt in 1..=2 {
        gate.send(Err(ProviderLaunchError::TimedOut("first execution".into())))
            .unwrap();
        loads_reach(&claude, attempt + 1);
        let catalog = service.snapshot();
        assert_eq!(catalog.providers[0].listing, ModelListing::Loading);
        assert_eq!(
            catalog.providers[0].availability,
            ProviderAvailability::Checking
        );
    }
    gate.send(Ok(listing("sonnet"))).unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    assert_eq!(claude.loads.load(Ordering::SeqCst), 3);
}

#[test]
fn exhausted_probe_timeouts_fail_but_the_next_read_retries_without_a_refresh() {
    let directory = tempfile::tempdir().unwrap();
    let (codex, gate) = gated(AgentChatProvider::Codex);
    let service = ModelCatalogService::new(
        vec![codex.clone()],
        ModelCatalogDefaults::open(directory.path()),
    )
    .with_retry(ProbeRetry {
        attempts: 2,
        delay: Duration::from_millis(1),
    });

    service.read(false);
    for _ in 0..2 {
        gate.send(Err(ProviderLaunchError::TimedOut("first execution".into())))
            .unwrap();
    }
    eventually(&service, |catalog| {
        matches!(catalog.providers[0].listing, ModelListing::Failed { .. })
    });
    assert_eq!(codex.loads.load(Ordering::SeqCst), 2);

    assert_eq!(
        service.read(false).providers[0].listing,
        ModelListing::Loading
    );
    gate.send(Ok(listing("gpt-6-astra"))).unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    assert_eq!(codex.loads.load(Ordering::SeqCst), 3);
}

struct InstalledSource {
    revision: std::sync::Mutex<Option<String>>,
    loads: AtomicUsize,
}

impl ModelCatalogSource for InstalledSource {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }

    fn label(&self) -> &'static str {
        "Codex"
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(600)
    }

    fn load(&self) -> Result<ProviderListing, ProviderLaunchError> {
        let loads = self.loads.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(ProviderListing {
            availability: if self.revision.lock().unwrap().is_some() {
                ProviderAvailability::Ready
            } else {
                ProviderAvailability::NotInstalled
            },
            models: vec![model(&format!("listing-{loads}"), None)],
        })
    }

    fn revision(&self) -> Option<String> {
        self.revision.lock().unwrap().clone()
    }
}

#[test]
fn an_installed_repaired_or_upgraded_provider_is_relisted_before_its_ttl() {
    let directory = tempfile::tempdir().unwrap();
    let source = Arc::new(InstalledSource {
        revision: std::sync::Mutex::new(None),
        loads: AtomicUsize::new(0),
    });
    let service = ModelCatalogService::new(
        vec![source.clone()],
        ModelCatalogDefaults::open(directory.path()),
    );
    let listed = |id: &str| {
        let id = id.to_owned();
        move |catalog: &gent_protocol::model_catalog::ModelCatalog| {
            catalog.providers[0].listing == ModelListing::Ready
                && catalog.providers[0].models[0].id == id
        }
    };
    service.read(false);
    eventually(&service, listed("listing-1"));
    assert_eq!(
        service.read(false).providers[0].availability,
        ProviderAvailability::NotInstalled
    );
    assert_eq!(source.loads.load(Ordering::SeqCst), 1);

    for (revision, listing) in [("installed", "listing-2"), ("upgraded", "listing-3")] {
        *source.revision.lock().unwrap() = Some(revision.into());
        assert_eq!(
            service.read(false).providers[0].listing,
            ModelListing::Loading
        );
        eventually(&service, listed(listing));
        assert_eq!(
            service.read(false).providers[0].availability,
            ProviderAvailability::Ready
        );
    }
    assert_eq!(source.loads.load(Ordering::SeqCst), 3);
}
