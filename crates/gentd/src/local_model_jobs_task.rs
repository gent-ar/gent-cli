use gent_protocol::{LocalModelDownloadFailure, LocalModelFrame, LocalModelInstallState};
use tokio::sync::{oneshot, watch};

use super::{DownloadProgress, LocalModelDownloadJobs, Outcome, PROGRESS_EVENT_PARTS, Stop};
use crate::{
    local_model_download::{ModelDownloadProgress, download_model},
    local_model_events::failure_for,
    local_model_provisioning::{LocalModelDownloadPlan, ModelInstallState},
};

impl LocalModelDownloadJobs {
    pub(super) async fn run(
        self,
        plan: LocalModelDownloadPlan,
        request_id: String,
        progress: watch::Sender<DownloadProgress>,
        stopped: oneshot::Receiver<Stop>,
    ) {
        let storage = self.storage_lock(&plan.model_id);
        let _storage = storage.lock().await;
        let outcome = tokio::select! {
            result = self.download(&plan, &request_id, &progress) => match result {
                Ok(()) => Outcome::Downloaded,
                Err(error) => Outcome::Failed(error),
            },
            stop = stopped => Outcome::Stopped(stop.unwrap_or(Stop::Detach)),
        };
        let terminal = match outcome {
            Outcome::Downloaded => DownloadProgress::Complete {
                size_bytes: plan.expected_bytes,
            },
            Outcome::Failed(_) if self.verified(&plan) => {
                let _ = std::fs::remove_file(&plan.partial_destination);
                DownloadProgress::Complete {
                    size_bytes: plan.expected_bytes,
                }
            }
            Outcome::Failed(error) => DownloadProgress::Failed(failure_for(&error)),
            Outcome::Stopped(Stop::Cancel) => {
                let _ = std::fs::remove_file(&plan.partial_destination);
                DownloadProgress::Failed(LocalModelDownloadFailure::Cancelled)
            }
            Outcome::Stopped(Stop::Detach) => {
                DownloadProgress::Failed(LocalModelDownloadFailure::Cancelled)
            }
        };
        self.finish(&plan.model_id, &request_id);
        if let DownloadProgress::Complete { size_bytes } = terminal {
            self.publish(LocalModelFrame::DownloadComplete {
                request_id,
                model_id: plan.model_id.clone(),
                size_bytes,
            });
        } else if let DownloadProgress::Failed(reason) = terminal {
            self.publish(LocalModelFrame::DownloadFailed {
                request_id,
                model_id: plan.model_id.clone(),
                reason,
            });
        }
        progress.send_replace(terminal);
    }

    async fn download(
        &self,
        plan: &LocalModelDownloadPlan,
        request_id: &str,
        progress: &watch::Sender<DownloadProgress>,
    ) -> Result<(), String> {
        let resumed = *progress.borrow();
        if let DownloadProgress::Downloading {
            downloaded_bytes,
            total_bytes,
        } = resumed
        {
            self.publish(LocalModelFrame::DownloadAccepted {
                request_id: request_id.to_owned(),
                model_id: plan.model_id.clone(),
                state: LocalModelInstallState::Downloading {
                    downloaded_bytes,
                    total_bytes,
                },
            });
        }
        self.provisioner
            .ensure_storage(plan)
            .map_err(|error| error.to_string())?;
        let mut published_bytes = 0;
        download_model(plan, self.transport.as_ref(), |event| {
            let (ModelDownloadProgress::Started {
                downloaded_bytes,
                total_bytes,
            }
            | ModelDownloadProgress::Advanced {
                downloaded_bytes,
                total_bytes,
            }) = event
            else {
                return;
            };
            progress.send_replace(DownloadProgress::Downloading {
                downloaded_bytes,
                total_bytes,
            });
            if downloaded_bytes == total_bytes
                || published_bytes == 0
                || downloaded_bytes - published_bytes >= total_bytes / PROGRESS_EVENT_PARTS
            {
                published_bytes = downloaded_bytes.max(1);
                self.publish(LocalModelFrame::DownloadProgress {
                    request_id: request_id.to_owned(),
                    model_id: plan.model_id.clone(),
                    downloaded_bytes,
                    total_bytes,
                });
            }
        })
        .await
        .map(drop)
        .map_err(|error| error.to_string())
    }

    fn verified(&self, plan: &LocalModelDownloadPlan) -> bool {
        matches!(
            self.provisioner.state(&plan.model_id),
            Ok(ModelInstallState::Ready { .. })
        )
    }

    fn finish(&self, model_id: &str, request_id: &str) {
        let mut jobs = self.jobs();
        if jobs
            .get(model_id)
            .is_some_and(|job| job.request_id == request_id)
        {
            jobs.remove(model_id);
        }
    }

    fn publish(&self, frame: LocalModelFrame) {
        if let Err(error) = self.events.publish(frame) {
            eprintln!("gentd could not record local model download progress: {error}");
        }
    }
}
