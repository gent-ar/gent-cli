use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

use gent_protocol::model_catalog::ModelListing;
use gent_types::AgentChatProvider;

use super::{
    ModelCatalogService, ModelCatalogSource, ProviderListing,
    tests::{GatedSource, eventually, gated, listing, loads_reach as wait_loads},
};
use crate::{
    provider_launch_budget::ProviderLaunchError,
    runtime_facade::model_catalog::defaults::ModelCatalogDefaults,
};

#[test]
fn repeated_refreshes_during_a_probe_coalesce_and_never_publish_the_obsolete_response() {
    let profile = tempfile::tempdir().unwrap();
    let (source, gate) = gated(AgentChatProvider::Codex);
    let service = ModelCatalogService::new(
        vec![source.clone()],
        ModelCatalogDefaults::open(profile.path()),
    );
    service.read(false);
    wait_loads(&source, 1);
    service.read(true);
    service.read(true);
    gate.send(Ok(listing("obsolete-provider-model"))).unwrap();
    wait_loads(&source, 2);
    assert_eq!(
        service.snapshot().providers[0].listing,
        ModelListing::Loading
    );
    assert!(service.snapshot().providers[0].models.is_empty());
    gate.send(Ok(listing("current-provider-model"))).unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    assert_eq!(
        service.snapshot().providers[0].models[0].id,
        "current-provider-model"
    );
    assert_eq!(source.loads.load(Ordering::SeqCst), 2);
}

struct RevisingSource {
    inner: Arc<GatedSource>,
    revision: Mutex<String>,
    ttl: Duration,
}

impl ModelCatalogSource for RevisingSource {
    fn provider(&self) -> AgentChatProvider {
        self.inner.provider()
    }
    fn label(&self) -> &'static str {
        self.inner.label()
    }
    fn ttl(&self) -> Duration {
        self.ttl
    }
    fn load(&self) -> Result<ProviderListing, ProviderLaunchError> {
        self.inner.load()
    }
    fn revision(&self) -> Option<String> {
        Some(self.revision.lock().unwrap().clone())
    }
}

#[test]
fn an_executable_change_during_discovery_cannot_stamp_an_old_catalog_with_the_new_ttl() {
    let profile = tempfile::tempdir().unwrap();
    let (inner, gate) = gated(AgentChatProvider::Codex);
    let source = Arc::new(RevisingSource {
        inner: inner.clone(),
        revision: Mutex::new("old-binary".into()),
        ttl: Duration::from_secs(600),
    });
    let service = ModelCatalogService::new(
        vec![source.clone()],
        ModelCatalogDefaults::open(profile.path()),
    );
    service.read(false);
    wait_loads(&inner, 1);
    *source.revision.lock().unwrap() = "new-binary".into();
    gate.send(Ok(listing("old-model"))).unwrap();
    wait_loads(&inner, 2);
    assert!(service.snapshot().providers[0].models.is_empty());
    gate.send(Ok(listing("new-model"))).unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    assert_eq!(service.snapshot().providers[0].models[0].id, "new-model");
}

#[test]
fn a_manual_refresh_during_a_background_reload_remains_loading_until_the_new_discovery_finishes() {
    let profile = tempfile::tempdir().unwrap();
    let (inner, gate) = gated(AgentChatProvider::Codex);
    let source = Arc::new(RevisingSource {
        inner: inner.clone(),
        revision: Mutex::new("same-binary".into()),
        ttl: Duration::ZERO,
    });
    let service =
        ModelCatalogService::new(vec![source], ModelCatalogDefaults::open(profile.path()));
    service.read(false);
    wait_loads(&inner, 1);
    gate.send(Ok(listing("previously-received"))).unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    assert_eq!(
        service.read(false).providers[0].listing,
        ModelListing::Ready
    );
    wait_loads(&inner, 2);
    assert_eq!(
        service.read(true).providers[0].listing,
        ModelListing::Loading
    );
    gate.send(Ok(listing("obsolete-background-response")))
        .unwrap();
    wait_loads(&inner, 3);
    let waiting = service.snapshot();
    assert_eq!(waiting.providers[0].listing, ModelListing::Loading);
    assert_eq!(waiting.providers[0].models[0].id, "previously-received");
    gate.send(Ok(listing("fresh-after-explicit-refresh")))
        .unwrap();
    eventually(&service, |catalog| {
        catalog.providers[0].listing == ModelListing::Ready
    });
    assert_eq!(
        service.snapshot().providers[0].models[0].id,
        "fresh-after-explicit-refresh"
    );
}
