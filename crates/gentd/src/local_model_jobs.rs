use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use gent_protocol::{LocalModelDownloadFailure, LocalModelInstallState};
use tokio::sync::{oneshot, watch};

use crate::{
    local_model_download::{ModelDownloadTransport, ReqwestModelDownloadTransport},
    local_model_events::{LocalModelEvents, failure_for},
    local_model_provisioning::{LocalModelDownloadPlan, LocalModelProvisioner, ModelInstallState},
};

const PROGRESS_EVENT_PARTS: u64 = 100;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DownloadHolder {
    Background,
    Prompt(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DownloadProgress {
    Downloading {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Complete {
        size_bytes: u64,
    },
    Failed(LocalModelDownloadFailure),
}

#[derive(Debug)]
pub(crate) enum DownloadStart {
    Ready { size_bytes: u64 },
    Downloading(watch::Receiver<DownloadProgress>),
}

#[derive(Clone, Debug)]
pub(crate) struct LocalModelDownloadJobs {
    provisioner: LocalModelProvisioner,
    transport: Arc<dyn ModelDownloadTransport>,
    events: LocalModelEvents,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    storage: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

#[derive(Debug)]
struct Job {
    request_id: String,
    holders: BTreeSet<DownloadHolder>,
    progress: watch::Receiver<DownloadProgress>,
    stop: oneshot::Sender<Stop>,
}

#[derive(Clone, Copy, Debug)]
enum Stop {
    Detach,
    Cancel,
}

enum Outcome {
    Downloaded,
    Failed(String),
    Stopped(Stop),
}

impl LocalModelDownloadJobs {
    pub(crate) fn new(provisioner: LocalModelProvisioner, events: LocalModelEvents) -> Self {
        Self {
            provisioner,
            transport: Arc::new(ReqwestModelDownloadTransport::new()),
            events,
            jobs: Arc::default(),
            storage: Arc::default(),
        }
    }

    pub(crate) fn with_transport(mut self, transport: Arc<dyn ModelDownloadTransport>) -> Self {
        self.transport = transport;
        self
    }

    pub(crate) fn install_state(
        &self,
        model_id: &str,
    ) -> Result<LocalModelInstallState, LocalModelDownloadFailure> {
        let plan = self.plan(model_id)?;
        if let Some(job) = self.jobs().get(model_id)
            && let DownloadProgress::Downloading {
                downloaded_bytes,
                total_bytes,
            } = *job.progress.borrow()
        {
            return Ok(LocalModelInstallState::Downloading {
                downloaded_bytes,
                total_bytes,
            });
        }
        match self.provisioner.state(model_id) {
            Ok(ModelInstallState::Ready { .. }) => Ok(LocalModelInstallState::Ready {
                size_bytes: plan.expected_bytes,
            }),
            Ok(_) => Ok(LocalModelInstallState::NotInstalled),
            Err(error) => Err(failure_for(&error.to_string())),
        }
    }

    pub(crate) fn start(
        &self,
        model_id: &str,
        holder: DownloadHolder,
    ) -> Result<DownloadStart, LocalModelDownloadFailure> {
        let plan = self.plan(model_id)?;
        let mut jobs = self.jobs();
        if let Some(job) = jobs.get_mut(model_id) {
            job.holders.insert(holder);
            return Ok(DownloadStart::Downloading(job.progress.clone()));
        }
        let downloaded_bytes = match self.provisioner.state(model_id) {
            Ok(ModelInstallState::Ready { .. }) => {
                return Ok(DownloadStart::Ready {
                    size_bytes: plan.expected_bytes,
                });
            }
            Ok(ModelInstallState::NotInstalled) => 0,
            Ok(ModelInstallState::Downloading { downloaded_bytes }) => downloaded_bytes,
            Err(error) => return Err(failure_for(&error.to_string())),
        };
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| LocalModelDownloadFailure::TransportFailed)?;
        let (sender, progress) = watch::channel(DownloadProgress::Downloading {
            downloaded_bytes,
            total_bytes: plan.expected_bytes,
        });
        let (stop, stopped) = oneshot::channel();
        let request_id = format!("model-download-{}", uuid::Uuid::new_v4());
        jobs.insert(
            model_id.to_owned(),
            Job {
                request_id: request_id.clone(),
                holders: BTreeSet::from([holder]),
                progress: progress.clone(),
                stop,
            },
        );
        runtime.spawn(self.clone().run(plan, request_id, sender, stopped));
        Ok(DownloadStart::Downloading(progress))
    }

    pub(crate) fn release(&self, model_id: &str, holder: &DownloadHolder) {
        let mut jobs = self.jobs();
        let Some(job) = jobs.get_mut(model_id) else {
            return;
        };
        job.holders.remove(holder);
        if job.holders.is_empty()
            && let Some(job) = jobs.remove(model_id)
        {
            let _ = job.stop.send(Stop::Detach);
        }
    }

    pub(crate) fn cancel(&self, model_id: &str) -> Result<bool, LocalModelDownloadFailure> {
        let plan = self.plan(model_id)?;
        if let Some(job) = self.jobs().remove(model_id) {
            let _ = job.stop.send(Stop::Cancel);
            return Ok(true);
        }
        match std::fs::remove_file(&plan.partial_destination) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(LocalModelDownloadFailure::StorageUnavailable),
        }
    }

    fn plan(&self, model_id: &str) -> Result<LocalModelDownloadPlan, LocalModelDownloadFailure> {
        self.provisioner
            .plan(model_id)
            .map_err(|_| LocalModelDownloadFailure::UnknownModel)
    }

    fn storage_lock(&self, model_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.storage
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(model_id.to_owned())
                .or_default(),
        )
    }

    fn jobs(&self) -> MutexGuard<'_, HashMap<String, Job>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[path = "local_model_jobs_task.rs"]
mod task;

#[cfg(test)]
#[path = "local_model_jobs_tests.rs"]
pub(crate) mod tests;
