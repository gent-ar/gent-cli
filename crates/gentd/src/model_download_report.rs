use gent_types::ModelDownloadPhase;

use crate::local_model_jobs::DownloadProgress;

const REPORT_PARTS: u64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DownloadUpdate {
    pub(crate) phase: ModelDownloadPhase,
    pub(crate) downloaded_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct DownloadReporter {
    last: Option<DownloadUpdate>,
}

impl DownloadReporter {
    pub(crate) fn observe(&mut self, progress: DownloadProgress) -> Option<DownloadUpdate> {
        let update = match progress {
            DownloadProgress::Downloading {
                downloaded_bytes,
                total_bytes,
            } => DownloadUpdate {
                phase: if downloaded_bytes >= total_bytes {
                    ModelDownloadPhase::Verifying
                } else {
                    ModelDownloadPhase::Downloading
                },
                downloaded_bytes,
                total_bytes: Some(total_bytes),
            },
            DownloadProgress::Complete { size_bytes } => DownloadUpdate {
                phase: ModelDownloadPhase::Ready,
                downloaded_bytes: size_bytes,
                total_bytes: Some(size_bytes),
            },
            DownloadProgress::Failed(_) => DownloadUpdate {
                phase: ModelDownloadPhase::Failed,
                downloaded_bytes: self.last.map_or(0, |last| last.downloaded_bytes),
                total_bytes: self.last.and_then(|last| last.total_bytes),
            },
        };
        let step = update
            .total_bytes
            .map_or(1, |total| (total / REPORT_PARTS).max(1));
        let due = self.last.is_none_or(|last| {
            last.phase != update.phase
                || update.downloaded_bytes >= last.downloaded_bytes.saturating_add(step)
        });
        due.then(|| {
            self.last = Some(update);
            update
        })
    }
}

#[cfg(test)]
#[path = "model_download_report_tests.rs"]
mod tests;
