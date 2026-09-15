use std::{path::Path, sync::Arc, time::Duration};

use gent_protocol::{LocalModelDownloadFailure, LocalModelFrame, LocalModelInstallState};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use super::{GatedChunks, download_frames, eventually, models};
use crate::{
    local_model_events::LocalModelEvents,
    local_model_jobs::{DownloadHolder, DownloadProgress, DownloadStart},
    standalone_authority_composition::StandaloneClaurstModels,
};

const MODEL: &str = "qwen3-8b-q4-k-m";
const CHUNK: usize = 1 << 20;

fn gated(directory: &Path) -> (SqliteLedger, Arc<GatedChunks>, StandaloneClaurstModels) {
    let ledger = SqliteLedger::open(directory.join("gent.db")).unwrap();
    let transport = Arc::new(GatedChunks::default());
    let models = StandaloneClaurstModels::from_data_dir(
        directory,
        LocalModelEvents::new(ledger.clone(), HostEpoch(1)),
    )
    .unwrap()
    .with_download_transport(Arc::new(Arc::clone(&transport)));
    (ledger, transport, models)
}

fn downloaded(models: &StandaloneClaurstModels) -> u64 {
    match models.install_state(MODEL).unwrap() {
        LocalModelInstallState::Downloading {
            downloaded_bytes, ..
        } => downloaded_bytes,
        _ => 0,
    }
}

fn verify_in_place(models: &StandaloneClaurstModels, model_id: &str) {
    let plan = models.provisioner.plan(model_id).unwrap();
    models.provisioner.ensure_storage(&plan).unwrap();
    std::fs::File::create(&plan.destination)
        .unwrap()
        .set_len(plan.expected_bytes)
        .unwrap();
    crate::local_model_integrity::remember_sha256(&plan.destination, &plan.expected_sha256);
}

#[tokio::test]
async fn every_holder_shares_one_transfer_and_one_announced_request() {
    let directory = tempfile::tempdir().unwrap();
    let (ledger, transport, models) = gated(directory.path());

    let first = models
        .downloads
        .start(MODEL, DownloadHolder::Background)
        .unwrap();
    let second = models
        .downloads
        .start(MODEL, DownloadHolder::Prompt("prompt".into()))
        .unwrap();
    assert!(matches!(first, DownloadStart::Downloading(_)));
    assert!(matches!(second, DownloadStart::Downloading(_)));
    transport.gate.notify_one();
    eventually(|| downloaded(&models) == CHUNK as u64).await;

    assert_eq!(*transport.ranges.lock().unwrap(), vec![None]);
    let frames = download_frames(&ledger);
    let request_ids = frames
        .iter()
        .map(|frame| match frame {
            LocalModelFrame::DownloadAccepted { request_id, .. }
            | LocalModelFrame::DownloadProgress { request_id, .. } => request_id.clone(),
            frame => panic!("unexpected {frame:?}"),
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(request_ids.len(), 1);
    assert!(matches!(
        frames[0],
        LocalModelFrame::DownloadAccepted { .. }
    ));
    models.downloads.cancel(MODEL).unwrap();
}

#[tokio::test]
async fn cancel_stops_the_transfer_removes_the_partial_and_reports_not_installed() {
    let directory = tempfile::tempdir().unwrap();
    let (ledger, transport, models) = gated(directory.path());
    let partial = models.provisioner.plan(MODEL).unwrap().partial_destination;
    let DownloadStart::Downloading(mut progress) = models
        .downloads
        .start(MODEL, DownloadHolder::Background)
        .unwrap()
    else {
        panic!("download starts");
    };
    transport.gate.notify_one();
    eventually(|| downloaded(&models) == CHUNK as u64).await;
    assert!(partial.is_file());

    assert!(models.downloads.cancel(MODEL).unwrap());

    assert_eq!(
        models.install_state(MODEL).unwrap(),
        LocalModelInstallState::NotInstalled
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        progress.wait_for(|state| !matches!(state, DownloadProgress::Downloading { .. })),
    )
    .await
    .expect("the download settles in time")
    .unwrap();
    assert_eq!(
        *progress.borrow(),
        DownloadProgress::Failed(LocalModelDownloadFailure::Cancelled)
    );
    assert!(!partial.exists());
    assert!(matches!(
        download_frames(&ledger).last(),
        Some(LocalModelFrame::DownloadFailed {
            reason: LocalModelDownloadFailure::Cancelled,
            ..
        })
    ));
    assert!(!models.downloads.cancel(MODEL).unwrap());
}

#[tokio::test]
async fn cancel_without_a_running_transfer_still_removes_a_left_behind_partial() {
    let directory = tempfile::tempdir().unwrap();
    let models = models(directory.path());
    let plan = models.provisioner.plan(MODEL).unwrap();
    models.provisioner.ensure_storage(&plan).unwrap();
    std::fs::write(&plan.partial_destination, [1_u8; 32]).unwrap();
    assert_eq!(
        models.install_state(MODEL).unwrap(),
        LocalModelInstallState::NotInstalled
    );

    assert!(models.downloads.cancel(MODEL).unwrap());

    assert!(!plan.partial_destination.exists());
}

#[tokio::test]
async fn the_last_holder_leaving_pauses_the_transfer_and_the_next_start_resumes_it() {
    let directory = tempfile::tempdir().unwrap();
    let (_ledger, transport, models) = gated(directory.path());
    let holder = DownloadHolder::Prompt("prompt".into());
    models.downloads.start(MODEL, holder.clone()).unwrap();
    transport.gate.notify_one();
    eventually(|| downloaded(&models) == CHUNK as u64).await;

    models.downloads.release(MODEL, &holder);

    assert_eq!(
        models.install_state(MODEL).unwrap(),
        LocalModelInstallState::NotInstalled
    );
    models
        .downloads
        .start(MODEL, DownloadHolder::Background)
        .unwrap();
    eventually(|| transport.ranges.lock().unwrap().len() == 2).await;
    assert_eq!(
        *transport.ranges.lock().unwrap(),
        vec![None, Some(CHUNK as u64)]
    );
    models.downloads.cancel(MODEL).unwrap();
}

#[tokio::test]
async fn a_verified_model_appearing_mid_transfer_completes_without_writing_the_partial_again() {
    let directory = tempfile::tempdir().unwrap();
    let (ledger, transport, models) = gated(directory.path());
    let plan = models.provisioner.plan(MODEL).unwrap();
    let DownloadStart::Downloading(mut progress) = models
        .downloads
        .start(MODEL, DownloadHolder::Background)
        .unwrap()
    else {
        panic!("download starts");
    };
    transport.gate.notify_one();
    eventually(|| downloaded(&models) == CHUNK as u64).await;

    verify_in_place(&models, MODEL);
    transport.gate.notify_one();

    tokio::time::timeout(
        Duration::from_secs(5),
        progress.wait_for(|state| !matches!(state, DownloadProgress::Downloading { .. })),
    )
    .await
    .expect("the download settles in time")
    .unwrap();
    assert_eq!(
        *progress.borrow(),
        DownloadProgress::Complete {
            size_bytes: plan.expected_bytes
        }
    );
    assert!(!plan.partial_destination.exists());
    assert!(matches!(
        download_frames(&ledger).last(),
        Some(LocalModelFrame::DownloadComplete { .. })
    ));
    assert_eq!(
        models.install_state(MODEL).unwrap(),
        LocalModelInstallState::Ready {
            size_bytes: plan.expected_bytes
        }
    );
}

#[tokio::test]
async fn a_verified_model_never_opens_a_transfer_or_a_partial() {
    let directory = tempfile::tempdir().unwrap();
    let (ledger, transport, models) = gated(directory.path());
    verify_in_place(&models, MODEL);

    assert!(matches!(
        models
            .downloads
            .start(MODEL, DownloadHolder::Background)
            .unwrap(),
        DownloadStart::Ready { .. }
    ));

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(transport.ranges.lock().unwrap().is_empty());
    assert!(
        !models
            .provisioner
            .plan(MODEL)
            .unwrap()
            .partial_destination
            .exists()
    );
    assert!(download_frames(&ledger).is_empty());
}
