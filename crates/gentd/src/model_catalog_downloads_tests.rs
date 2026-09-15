use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use gent_protocol::model_catalog::ModelCatalogSelection;
use gent_types::AgentChatProvider;
use tokio::sync::Notify;

use super::LocalModelDownloads;
use crate::{
    local_model_download::{
        DownloadRequest, ModelDownloadError, ModelDownloadResponse, ModelDownloadTransport,
    },
    runtime_facade::model_catalog::defaults::ModelCatalogDefaults,
    standalone_authority_composition::StandaloneClaurstModels,
};

#[derive(Debug, Default)]
pub(crate) struct GatedTransport {
    pub(crate) requests: AtomicUsize,
    pub(crate) finish: Notify,
}

#[derive(Debug)]
struct GatedResponse(Arc<GatedTransport>);

#[async_trait]
impl ModelDownloadTransport for Arc<GatedTransport> {
    async fn get(
        &self,
        _: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(GatedResponse(Arc::clone(self))))
    }
}

#[async_trait]
impl ModelDownloadResponse for GatedResponse {
    fn status(&self) -> u16 {
        200
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        self.0.finish.notified().await;
        Err(ModelDownloadError::Transport(
            "fake transfer finished".into(),
        ))
    }
}

pub(crate) fn complete_small_model(models: &StandaloneClaurstModels) {
    let plan = models.provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    models.provisioner.ensure_storage(&plan).unwrap();
    fs::File::create(&plan.destination)
        .unwrap()
        .set_len(plan.expected_bytes)
        .unwrap();
    crate::local_model_integrity::remember_sha256(&plan.destination, &plan.expected_sha256);
}

pub(crate) async fn transfer_started(transport: &GatedTransport) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while transport.requests.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

pub(crate) fn downloading(models: &StandaloneClaurstModels, model_id: &str) -> bool {
    matches!(
        models.install_state(model_id).unwrap(),
        gent_protocol::LocalModelInstallState::Downloading { .. }
    )
}

async fn settle(models: &StandaloneClaurstModels, model_id: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while downloading(models, model_id) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

fn downloads(
    directory: &std::path::Path,
    transport: &Arc<GatedTransport>,
) -> (StandaloneClaurstModels, LocalModelDownloads) {
    let models = crate::local_model_jobs::tests::models(directory)
        .with_download_transport(Arc::new(Arc::clone(transport)));
    let downloads = LocalModelDownloads::new(models.clone(), ModelCatalogDefaults::open(directory));
    (models, downloads)
}

#[tokio::test]
async fn first_launch_downloads_the_default_once_and_a_restart_does_not_download_again() {
    let directory = tempfile::tempdir().unwrap();
    let transport = Arc::new(GatedTransport::default());
    let (models, first) = downloads(directory.path(), &transport);

    assert!(first.start_default_when_missing().unwrap());
    assert!(first.start_default_when_missing().unwrap());
    assert!(downloading(&models, "qwen3-1-7b-q4-k-m"));

    transfer_started(&transport).await;
    let plan = models.provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    assert!(plan.partial_destination.is_file());
    complete_small_model(&models);
    transport.finish.notify_one();
    settle(&models, "qwen3-1-7b-q4-k-m").await;
    assert_eq!(transport.requests.load(Ordering::SeqCst), 1);

    let (_, restarted) = downloads(directory.path(), &transport);
    assert!(!restarted.start_default_when_missing().unwrap());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(transport.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancel_stops_the_background_download_and_is_remembered_until_explicitly_started() {
    let directory = tempfile::tempdir().unwrap();
    let transport = Arc::new(GatedTransport::default());
    let (models, first) = downloads(directory.path(), &transport);
    assert!(first.start_default_when_missing().unwrap());

    assert!(first.cancel("qwen3-1-7b-q4-k-m").unwrap());
    assert!(!downloading(&models, "qwen3-1-7b-q4-k-m"));
    assert!(!first.cancel("qwen3-1-7b-q4-k-m").unwrap());

    let (models, restarted) = downloads(directory.path(), &transport);
    assert!(!restarted.start_default_when_missing().unwrap());
    assert!(!downloading(&models, "qwen3-1-7b-q4-k-m"));

    assert!(restarted.start("qwen3-1-7b-q4-k-m").unwrap());
    assert!(downloading(&models, "qwen3-1-7b-q4-k-m"));
    assert!(!ModelCatalogDefaults::open(directory.path()).declined("qwen3-1-7b-q4-k-m"));
    restarted.cancel("qwen3-1-7b-q4-k-m").unwrap();
}

#[tokio::test]
async fn a_hosted_default_never_starts_a_local_download() {
    let directory = tempfile::tempdir().unwrap();
    ModelCatalogDefaults::open(directory.path())
        .save_selection(ModelCatalogSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-6-astra".into(),
        })
        .unwrap();
    let transport = Arc::new(GatedTransport::default());
    let (models, downloads) = downloads(directory.path(), &transport);
    assert!(!downloads.start_default_when_missing().unwrap());
    assert!(!downloading(&models, "qwen3-1-7b-q4-k-m"));
}
