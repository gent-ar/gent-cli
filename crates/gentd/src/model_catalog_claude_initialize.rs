use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, TryLockError},
    time::{Duration, Instant, SystemTime},
};

use gent_drivers::{PublicProvider, launch_spec::LaunchIntent};
use gent_types::AgentChatProvider;
use serde_json::{Value, json};

use super::probe::{ProbeStep, ProviderProbe};
use crate::provider_launch_budget::ProviderLaunchError;

const INITIALIZE_REQUEST_ID: &str = "gent-model-catalog";
const LISTED_TTL: Duration = Duration::from_secs(600);
const FAILED_TTL: Duration = Duration::from_secs(30);

type SlotKey = (Option<PathBuf>, PathBuf, u64, Option<SystemTime>);

#[derive(Default)]
struct Slot {
    loaded_at: Option<Instant>,
    response: Option<Result<Value, ProviderLaunchError>>,
}

impl Slot {
    fn fresh(&self, failures: bool) -> bool {
        let ttl = match self.response {
            Some(Ok(_)) => LISTED_TTL,
            Some(Err(_)) if failures => FAILED_TTL,
            _ => return false,
        };
        self.loaded_at.is_some_and(|loaded| loaded.elapsed() < ttl)
    }
}

pub(crate) enum InitializeState {
    NotInstalled,
    Loading,
    Listed(Value),
    Failed(String),
}

pub(crate) struct ClaudeInitialize {
    executables: crate::provider_executables::ProviderExecutables,
    timeout: Duration,
    slots: Mutex<BTreeMap<SlotKey, Arc<Mutex<Slot>>>>,
}

impl ClaudeInitialize {
    pub(crate) fn new(
        executables: crate::provider_executables::ProviderExecutables,
        timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            executables,
            timeout,
            slots: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn revision(&self) -> Option<String> {
        self.executables.revision(AgentChatProvider::Claude)
    }

    pub(crate) fn installed(&self) -> bool {
        self.executables
            .executable(AgentChatProvider::Claude)
            .is_some()
    }

    pub(crate) fn load(
        &self,
        workspace: Option<&Path>,
        refresh: bool,
    ) -> Result<Value, ProviderLaunchError> {
        let Some((executable, slot)) = self.slot(workspace) else {
            return Err(ProviderLaunchError::Failed(
                "claude is not installed".into(),
            ));
        };
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if refresh || !slot.fresh(false) {
            slot.response = Some(self.probe(executable, workspace));
            slot.loaded_at = Some(Instant::now());
        }
        slot.response
            .clone()
            .unwrap_or_else(|| Err("claude did not answer initialize".to_owned().into()))
    }

    pub(crate) fn state(
        self: &Arc<Self>,
        workspace: Option<PathBuf>,
        refresh: bool,
    ) -> InitializeState {
        let Some((_, slot)) = self.slot(workspace.as_deref()) else {
            return InitializeState::NotInstalled;
        };
        let guard = match slot.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return InitializeState::Loading,
        };
        let listed = guard
            .response
            .clone()
            .filter(|_| guard.fresh(true) && !refresh);
        drop(guard);
        match listed {
            Some(Ok(response)) => InitializeState::Listed(response),
            Some(Err(error)) => InitializeState::Failed(error.to_string()),
            None => {
                let initialize = Arc::clone(self);
                std::thread::spawn(move || initialize.load(workspace.as_deref(), refresh));
                InitializeState::Loading
            }
        }
    }

    fn slot(&self, workspace: Option<&Path>) -> Option<(PathBuf, Arc<Mutex<Slot>>)> {
        let executable = self.executables.executable(AgentChatProvider::Claude)?;
        let metadata = std::fs::metadata(&executable).ok();
        let key = (
            workspace.map(Path::to_path_buf),
            executable.clone(),
            metadata.as_ref().map_or(0, std::fs::Metadata::len),
            metadata.and_then(|metadata| metadata.modified().ok()),
        );
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        Some((executable, Arc::clone(slots.entry(key).or_default())))
    }

    fn probe(
        &self,
        executable: PathBuf,
        workspace: Option<&Path>,
    ) -> Result<Value, ProviderLaunchError> {
        let arguments = gent_drivers::launch_spec::arguments("claude", &LaunchIntent::Start)
            .map_err(|error| error.to_string())?;
        let initialize = json!({
            "type": "control_request",
            "request_id": INITIALIZE_REQUEST_ID,
            "request": {"subtype": "initialize"},
        });
        let mut frame = serde_json::to_vec(&initialize).map_err(|error| error.to_string())?;
        frame.push(b'\n');
        ProviderProbe {
            provider: PublicProvider::Claude,
            executable,
            arguments,
            timeout: self.timeout,
            workspace_root: workspace.map(Path::to_path_buf),
        }
        .exchange(&[frame], |value, _| Ok(initialize_response(value)))?
        .map_err(ProviderLaunchError::Failed)
    }
}

pub(crate) fn initialize_response(frame: &Value) -> ProbeStep<Result<Value, String>> {
    if frame.get("type").and_then(Value::as_str) != Some("control_response") {
        return ProbeStep::Continue;
    }
    let response = &frame["response"];
    if response.get("request_id").and_then(Value::as_str) != Some(INITIALIZE_REQUEST_ID) {
        return ProbeStep::Continue;
    }
    if response.get("subtype").and_then(Value::as_str) != Some("success") {
        return ProbeStep::Done(Err("claude refused to initialize".into()));
    }
    ProbeStep::Done(Ok(response["response"].clone()))
}
