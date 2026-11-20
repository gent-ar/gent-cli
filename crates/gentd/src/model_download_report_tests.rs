use gent_protocol::LocalModelDownloadFailure;
use gent_types::ModelDownloadPhase;

use super::DownloadReporter;
use crate::local_model_jobs::DownloadProgress;

fn downloading(downloaded_bytes: u64) -> DownloadProgress {
    DownloadProgress::Downloading {
        downloaded_bytes,
        total_bytes: 1000,
    }
}

#[test]
fn progress_reports_at_most_once_per_hundredth_and_on_every_phase_change() {
    let mut reporter = DownloadReporter::default();
    let updates = [0, 5, 9, 10, 15, 20, 999, 1000]
        .into_iter()
        .filter_map(|bytes| reporter.observe(downloading(bytes)))
        .map(|update| (update.phase, update.downloaded_bytes))
        .collect::<Vec<_>>();
    assert_eq!(
        updates,
        vec![
            (ModelDownloadPhase::Downloading, 0),
            (ModelDownloadPhase::Downloading, 10),
            (ModelDownloadPhase::Downloading, 20),
            (ModelDownloadPhase::Downloading, 999),
            (ModelDownloadPhase::Verifying, 1000),
        ]
    );
    let ready = reporter
        .observe(DownloadProgress::Complete { size_bytes: 1000 })
        .unwrap();
    assert_eq!(ready.phase, ModelDownloadPhase::Ready);
    assert_eq!(ready.total_bytes, Some(1000));
}

#[test]
fn a_failure_keeps_the_last_reported_bytes() {
    let mut reporter = DownloadReporter::default();
    reporter.observe(downloading(400));
    let failed = reporter
        .observe(DownloadProgress::Failed(LocalModelDownloadFailure::Stalled))
        .unwrap();
    assert_eq!(failed.phase, ModelDownloadPhase::Failed);
    assert_eq!(failed.downloaded_bytes, 400);
    assert_eq!(failed.total_bytes, Some(1000));
}
