use std::sync::Arc;

use gent_protocol::{
    LOCAL_MODELS_CAPABILITY, LocalModelDownloadFailure, LocalModelFrame, LocalModelInstallState,
    read_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::json;
use tokio::io::duplex;

use super::{LocalModelPort, dispatch};
use crate::{
    local_model_jobs::{
        DownloadHolder, DownloadStart,
        tests::{GatedChunks, eventually, models},
    },
    standalone_authority_composition::StandaloneClaurstModels,
};

const MODEL: &str = "qwen3-8b-q4-k-m";

struct Port(StandaloneClaurstModels);

impl LocalModelPort for Port {
    fn local_models(&self, _: LocalModelFrame) -> Result<LocalModelFrame, String> {
        Err("unused".into())
    }

    fn start_local_model_download(
        &self,
        model_id: &str,
    ) -> Result<DownloadStart, LocalModelDownloadFailure> {
        self.0.downloads.start(model_id, DownloadHolder::Background)
    }
}

fn request(model_id: &str) -> serde_json::Value {
    json!({"type":"download","body":{"requestId":"download-1","modelId":model_id}})
}

#[tokio::test]
async fn a_streamed_download_follows_the_shared_job_to_its_terminal_frame() {
    let directory = tempfile::tempdir().unwrap();
    let transport = Arc::new(GatedChunks::default());
    let models = models(directory.path()).with_download_transport(Arc::new(Arc::clone(&transport)));
    let port = Port(models.clone());
    let (mut client, mut server) = duplex(1 << 16);
    let capabilities = CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]);
    let stream = tokio::spawn(async move {
        dispatch(&mut server, &port, &capabilities, &request(MODEL))
            .await
            .unwrap()
    });

    let accepted = read_json_frame::<_, LocalModelFrame>(&mut client)
        .await
        .unwrap();
    assert!(matches!(
        accepted,
        LocalModelFrame::DownloadAccepted {
            state: LocalModelInstallState::Downloading {
                downloaded_bytes: 0,
                ..
            },
            ..
        }
    ));
    transport.gate.notify_one();
    let progress = read_json_frame::<_, LocalModelFrame>(&mut client)
        .await
        .unwrap();
    assert!(matches!(
        progress,
        LocalModelFrame::DownloadProgress { downloaded_bytes, .. } if downloaded_bytes == 1 << 20
    ));
    models.downloads.cancel(MODEL).unwrap();
    let terminal = read_json_frame::<_, LocalModelFrame>(&mut client)
        .await
        .unwrap();
    assert!(matches!(
        terminal,
        LocalModelFrame::DownloadFailed {
            reason: LocalModelDownloadFailure::Cancelled,
            ref request_id,
            ..
        } if request_id == "download-1"
    ));
    assert!(stream.await.unwrap());
}

#[tokio::test]
async fn an_unknown_model_is_a_typed_terminal_failure_without_a_transfer() {
    let directory = tempfile::tempdir().unwrap();
    let transport = Arc::new(GatedChunks::default());
    let port =
        Port(models(directory.path()).with_download_transport(Arc::new(Arc::clone(&transport))));
    let (mut client, mut server) = duplex(4096);
    let capabilities = CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]);
    dispatch(&mut server, &port, &capabilities, &request("not-curated"))
        .await
        .unwrap();
    assert!(matches!(
        read_json_frame::<_, LocalModelFrame>(&mut client)
            .await
            .unwrap(),
        LocalModelFrame::DownloadFailed {
            reason: LocalModelDownloadFailure::UnknownModel,
            ..
        }
    ));
    assert!(transport.ranges.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_disconnected_client_leaves_the_daemon_owned_download_running() {
    let directory = tempfile::tempdir().unwrap();
    let transport = Arc::new(GatedChunks::default());
    let models = models(directory.path()).with_download_transport(Arc::new(Arc::clone(&transport)));
    let port = Port(models.clone());
    let (client, mut server) = duplex(4096);
    drop(client);
    let capabilities = CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]);
    assert!(
        dispatch(&mut server, &port, &capabilities, &request(MODEL))
            .await
            .is_err()
    );
    transport.gate.notify_one();
    eventually(|| {
        models.install_state(MODEL).unwrap()
            == LocalModelInstallState::Downloading {
                downloaded_bytes: 1 << 20,
                total_bytes: models.provisioner.plan(MODEL).unwrap().expected_bytes,
            }
    })
    .await;
    models.downloads.cancel(MODEL).unwrap();
}
