//! Run-bound serial model actor. No public IPC or client-provided authority.
use gent_types::SandboxNetworkPolicy;
use std::{
    io::Write,
    sync::{
        Arc,
        mpsc::{self, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
};
#[path = "contained_model_protocol.rs"]
mod protocol;
#[path = "contained_model_scope.rs"]
mod scope;
pub(crate) use protocol::ModelRequest;
use protocol::{Metadata, Reader};
#[cfg(any(windows, test))]
pub(crate) use scope::ModelResources;
use scope::Scope;
#[path = "contained_model_budget.rs"]
mod budget;
#[path = "contained_model_exchange.rs"]
mod exchange;
#[cfg(windows)]
#[path = "contained_model_windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{ModelRun, SelectedModel, WindowsModelFactory};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelError {
    Request,
    HelperManifestMismatch,
    Protocol,
    Transport,
    Timeout,
    IdleTimeout,
    TotalTimeout,
    Revoked,
    Busy,
    Unknown,
}
impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "contained model {self:?}")
    }
}
impl std::error::Error for ModelError {}
enum Event {
    Metadata(Metadata),
    Data(Vec<u8>),
    End,
    Error(ModelError),
}
struct Job {
    encoded: Vec<u8>,
    events: SyncSender<Event>,
}
#[derive(Clone)]
pub(crate) struct ModelEndpoint {
    scope: Arc<Scope>,
    requests: SyncSender<Job>,
    sdk_lock: Option<gent_types::RunVersionLock>,
}
impl std::fmt::Debug for ModelEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelEndpoint")
            .field("identity", &self.identity())
            .finish_non_exhaustive()
    }
}
pub(crate) struct ModelResponseError {
    pub started: bool,
    pub error: ModelError,
}
impl ModelEndpoint {
    fn start(scope: Arc<Scope>, nonce: String) -> Result<Self, ModelError> {
        if scope.model_digest.len() != 64
            || !scope
                .model_digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || scope.model_id.is_empty()
            || scope.model_id.len() > 128
            || scope.request.profile.network() != &SandboxNetworkPolicy::Disabled
            || scope.request.profile.gateway().is_some()
            || nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !scope.current()
        {
            return Err(ModelError::Revoked);
        }
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let actor = scope.clone();
        std::thread::Builder::new()
            .name("gent-model".into())
            .spawn(move || {
                let mut reader = Reader::new(actor.clone());
                let mut intro = b"GENTSR01".to_vec();
                intro.extend(nonce.as_bytes());
                let result = actor
                    .write(&intro)
                    .and_then(|()| reader.exact(40, Instant::now() + Duration::from_secs(300)))
                    .and_then(|v| {
                        if v == [b"GENTSD01".as_slice(), nonce.as_bytes()].concat() {
                            Ok(())
                        } else {
                            Err(ModelError::Protocol)
                        }
                    });
                if ready_tx.send(result).is_err() || result.is_err() {
                    let _ = actor.retire();
                    return;
                }
                loop {
                    if !actor.current() {
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(job) => {
                            let result = exchange::run(&actor, &mut reader, &job);
                            if let Err(error) = result {
                                let _ = job.events.try_send(Event::Error(error));
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let _ = actor.retire();
            })
            .map_err(|_| ModelError::Transport)?;
        match ready_rx.recv_timeout(Duration::from_secs(305)) {
            Ok(Ok(())) => Ok(Self {
                scope,
                requests: tx,
                sdk_lock: None,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let _ = scope.retire();
                Err(ModelError::Timeout)
            }
        }
    }
    pub(crate) fn respond(
        &self,
        request: ModelRequest,
        out: &mut impl Write,
    ) -> Result<(), ModelResponseError> {
        let mut started = false;
        let result = (|| {
            if !self.current() {
                return Err(ModelError::Revoked);
            }
            if request.method == "POST" {
                let body: serde_json::Value =
                    serde_json::from_slice(&request.body).map_err(|_| ModelError::Request)?;
                if body.get("model").and_then(|v| v.as_str()) != Some(self.scope.model_id.as_str())
                {
                    return Err(ModelError::Request);
                }
            }
            let encoded = request.encode()?;
            let (tx, rx) = mpsc::sync_channel(2);
            self.requests
                .try_send(Job {
                    encoded,
                    events: tx,
                })
                .map_err(|e| match e {
                    TrySendError::Full(_) => ModelError::Busy,
                    TrySendError::Disconnected(_) => ModelError::Transport,
                })?;
            let deadline = Instant::now() + Duration::from_secs(300);
            let mut metadata = false;
            loop {
                if !self.current() {
                    return Err(ModelError::Revoked);
                }
                if Instant::now() >= deadline {
                    return Err(ModelError::TotalTimeout);
                }
                match rx.recv_timeout(Duration::from_millis(25)) {
                    Ok(Event::Metadata(value)) => {
                        if metadata {
                            return Err(ModelError::Protocol);
                        }
                        metadata = true;
                        started = true;
                        value.write(out)?;
                    }
                    Ok(Event::Data(bytes)) => {
                        if !metadata {
                            return Err(ModelError::Protocol);
                        }
                        protocol::chunk(out, &bytes)?
                    }
                    Ok(Event::End) => {
                        if !metadata {
                            return Err(ModelError::Protocol);
                        }
                        out.write_all(b"0\r\n\r\n")
                            .and_then(|()| out.flush())
                            .map_err(|_| ModelError::Transport)?;
                        return Ok(());
                    }
                    Ok(Event::Error(error)) => return Err(error),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err(ModelError::Transport),
                }
            }
        })();
        result.map_err(|error| {
            if !matches!(error, ModelError::Busy | ModelError::Request) {
                eprintln!("{error} retired run {}", self.scope.run);
                let _ = self.revoke();
            }
            ModelResponseError { started, error }
        })
    }
}

#[cfg(test)]
#[path = "contained_model_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "contained_model_native_tests.rs"]
pub(crate) mod native_tests;

#[path = "contained_model_access.rs"]
mod access;

#[cfg(test)]
#[path = "contained_model_stream_tests.rs"]
mod stream_tests;

#[path = "contained_model_admission.rs"]
mod admission;
#[cfg(windows)]
pub(crate) use admission::packaged_model_error;
