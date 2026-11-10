use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use gent_ports::Ledger;
use gent_protocol::LocalModelFrame;
use gent_store::SqliteLedger;
use gent_types::HostEpoch;
use tokio::sync::Notify;

use crate::{
    local_model_download::{
        DownloadRequest, ModelDownloadError, ModelDownloadResponse, ModelDownloadTransport,
    },
    local_model_events::LocalModelEvents,
    standalone_authority_composition::StandaloneClaurstModels,
};

const CHUNK: usize = 1 << 20;

#[derive(Debug)]
pub(crate) struct BlockingTransport;

#[derive(Debug)]
struct BlockingResponse;

#[async_trait]
impl ModelDownloadTransport for BlockingTransport {
    async fn get(
        &self,
        _: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        Ok(Box::new(BlockingResponse))
    }
}

#[async_trait]
impl ModelDownloadResponse for BlockingResponse {
    fn status(&self) -> u16 {
        200
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        std::future::pending().await
    }
}

#[derive(Debug, Default)]
pub(crate) struct GatedChunks {
    pub(crate) ranges: Mutex<Vec<Option<u64>>>,
    pub(crate) gate: Notify,
}

#[derive(Debug)]
struct GatedChunk(Arc<GatedChunks>, u16);

#[async_trait]
impl ModelDownloadTransport for Arc<GatedChunks> {
    async fn get(
        &self,
        request: DownloadRequest,
    ) -> Result<Box<dyn ModelDownloadResponse>, ModelDownloadError> {
        self.ranges.lock().unwrap().push(request.range_start);
        let status = if request.range_start.is_some() {
            206
        } else {
            200
        };
        Ok(Box::new(GatedChunk(Arc::clone(self), status)))
    }
}

#[async_trait]
impl ModelDownloadResponse for GatedChunk {
    fn status(&self) -> u16 {
        self.1
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ModelDownloadError> {
        self.0.gate.notified().await;
        Ok(Some(vec![0; CHUNK]))
    }
}

pub(crate) fn models(directory: &Path) -> StandaloneClaurstModels {
    placeholder_models(
        directory,
        LocalModelEvents::new(SqliteLedger::in_memory().unwrap(), HostEpoch(1)),
    )
}

pub(crate) fn placeholder_models(
    directory: &Path,
    events: LocalModelEvents,
) -> StandaloneClaurstModels {
    let mut catalog: serde_json::Value =
        serde_json::from_str(include_str!("../models.json")).unwrap();
    for model in catalog["models"].as_array_mut().unwrap() {
        model["size_bytes"] = (4 * CHUNK).into();
    }
    StandaloneClaurstModels::with_catalog(
        directory,
        crate::local_model_catalog::LocalModelCatalog::from_json(&catalog.to_string()).unwrap(),
        events,
    )
}

pub(crate) async fn eventually(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition holds in time");
}

pub(crate) fn download_frames(ledger: &SqliteLedger) -> Vec<LocalModelFrame> {
    ledger
        .read_event_page(0, 1000)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "localModelDownload")
        .map(|event| serde_json::from_value(event.payload).unwrap())
        .collect()
}

#[path = "local_model_jobs_behavior_tests.rs"]
mod behavior;

pub(crate) fn in_temp_dir<F, Fut>(body: F)
where
    F: FnOnce(std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let directory = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(body(directory.path().to_path_buf()));
    runtime.shutdown_timeout(Duration::from_secs(5));
    directory.close().unwrap();
}
