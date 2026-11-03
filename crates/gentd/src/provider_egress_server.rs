use std::collections::BTreeMap;
use std::io::BufReader;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::{EgressError, ProviderApi, ReviewedProviderEgress, wire};

type Connections = Arc<Mutex<BTreeMap<u64, TcpStream>>>;
type Mcp = Arc<Mutex<Option<Arc<dyn super::mcp_http::McpEndpoint>>>>;

pub struct ProviderEgressBroker {
    endpoint: SocketAddr,
    token: String,
    policy_digest: String,
    api: ProviderApi,
    stopped: Arc<AtomicBool>,
    connections: Connections,
    acceptor: Option<JoinHandle<()>>,
    mcp: Mcp,
}

impl std::fmt::Debug for ProviderEgressBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderEgressBroker")
            .field("endpoint", &self.endpoint)
            .field("policy_digest", &self.policy_digest)
            .finish_non_exhaustive()
    }
}

impl ProviderEgressBroker {
    #[cfg(all(test, windows))]
    pub fn start(policy: ReviewedProviderEgress) -> Result<Self, EgressError> {
        Self::start_checked(policy, Arc::new(|| true))
    }

    pub fn start_checked(
        policy: ReviewedProviderEgress,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, EgressError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| EgressError::Transport)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| EgressError::Transport)?;
        let endpoint = listener.local_addr().map_err(|_| EgressError::Transport)?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let stopped = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(BTreeMap::new()));
        let mcp: Mcp = Arc::new(Mutex::new(None));
        let policy_digest = policy.policy_digest().to_string();
        let api = policy.api;
        let acceptor = {
            let stopped = stopped.clone();
            let connections = connections.clone();
            let token = token.clone();
            let mcp = mcp.clone();
            thread::spawn(move || {
                accept(listener, policy, token, stopped, connections, current, mcp)
            })
        };
        Ok(Self {
            endpoint,
            token,
            policy_digest,
            api,
            stopped,
            connections,
            acceptor: Some(acceptor),
            mcp,
        })
    }

    #[cfg(all(test, windows))]
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    /// Called once by the run owner before the contained provider is resumed.
    pub(super) fn attach_mcp(
        &self,
        bridge: Arc<dyn super::mcp_http::McpEndpoint>,
    ) -> Result<(), EgressError> {
        if !matches!(self.api, ProviderApi::Claude | ProviderApi::Claurst)
            || self.stopped.load(Ordering::Acquire)
        {
            return Err(EgressError::Configuration);
        }
        let mut held = self.mcp.lock().map_err(|_| EgressError::Configuration)?;
        if held.is_some() {
            return Err(EgressError::Configuration);
        }
        *held = Some(bridge);
        Ok(())
    }

    pub(crate) fn mcp_settled(&self) -> bool {
        self.mcp
            .lock()
            .is_ok_and(|held| held.as_ref().is_none_or(|b| b.settled()))
    }

    #[cfg(all(test, windows))]
    pub fn client_base_url(&self) -> String {
        format!("http://{}/v1", self.endpoint)
    }

    pub fn client_token(&self) -> &str {
        &self.token
    }

    pub fn gateway_binding(&self) -> Result<gent_types::SandboxProviderGateway, EgressError> {
        let SocketAddr::V4(endpoint) = self.endpoint else {
            return Err(EgressError::Configuration);
        };
        let api = match self.api {
            ProviderApi::Claude => gent_types::SandboxGatewayApi::Claude,
            #[cfg(windows)]
            ProviderApi::Codex => gent_types::SandboxGatewayApi::Codex,
            ProviderApi::Claurst => gent_types::SandboxGatewayApi::Claurst,
        };
        gent_types::SandboxProviderGateway::new(
            api,
            endpoint,
            self.token.clone(),
            self.policy_digest.clone(),
        )
        .map_err(|_| EgressError::Configuration)
    }

    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }

    pub fn revoke(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(held) = self.mcp.lock() {
            if let Some(bridge) = held.as_ref() {
                bridge.revoke();
            }
        }
        if let Ok(connections) = self.connections.lock() {
            for stream in connections.values() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

impl Drop for ProviderEgressBroker {
    fn drop(&mut self) {
        self.revoke();
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
        }
        if let Ok(connections) = self.connections.lock() {
            for stream in connections.values() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

fn accept(
    listener: TcpListener,
    policy: ReviewedProviderEgress,
    token: String,
    stopped: Arc<AtomicBool>,
    connections: Connections,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
    mcp: Mcp,
) {
    let policy = Arc::new(policy);
    let token = Arc::new(token);
    let sequence = AtomicU64::new(0);
    while !stopped.load(Ordering::Acquire) {
        let Ok((mut stream, peer)) = listener.accept() else {
            thread::sleep(Duration::from_millis(10));
            continue;
        };
        if !peer.ip().is_loopback() || stopped.load(Ordering::Acquire) {
            continue;
        }
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        let Ok(clone) = stream.try_clone() else {
            continue;
        };
        let id = sequence.fetch_add(1, Ordering::Relaxed);
        let Ok(mut active) = connections.lock() else {
            break;
        };
        if active.len() >= 8 {
            wire::reject(&mut stream, &EgressError::Transport);
            continue;
        }
        active.insert(id, clone);
        drop(active);
        let connections = connections.clone();
        let policy = policy.clone();
        let token = token.clone();
        let stopped = stopped.clone();
        let current = current.clone();
        let mcp = mcp.clone();
        thread::spawn(move || {
            let result = serve(&mut stream, &policy, &token, &stopped, &*current, &mcp);
            if let Err(error) = result {
                wire::reject(&mut stream, &error);
            }
            let _ = stream.shutdown(Shutdown::Both);
            if let Ok(mut active) = connections.lock() {
                active.remove(&id);
            }
        });
    }
}

fn serve(
    stream: &mut TcpStream,
    policy: &ReviewedProviderEgress,
    token: &str,
    stopped: &AtomicBool,
    current: &(dyn Fn() -> bool + Send + Sync),
    mcp: &Mcp,
) -> Result<(), EgressError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| EgressError::Transport)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| EgressError::Transport)?;
    let authority = stream
        .local_addr()
        .map_err(|_| EgressError::Transport)?
        .to_string();
    let request = wire::read(&mut BufReader::new(&mut *stream), &authority, token)?;
    if stopped.load(Ordering::Acquire) || !current() {
        return Err(EgressError::Authentication);
    }
    if request.path.starts_with("/__gent_mcp/") {
        let bridge = mcp
            .lock()
            .map_err(|_| EgressError::Configuration)?
            .clone()
            .ok_or(EgressError::Configuration)?;
        return bridge
            .respond(request, stream)
            .map_err(|_| EgressError::Transport);
    }
    #[cfg(windows)]
    if let super::ProviderTarget::ContainedModel(model) = &policy.target {
        let response = model.respond(
            crate::contained_model::ModelRequest {
                method: request.method,
                path: request.path,
                body: request.body,
            },
            stream,
        );
        return match response {
            Ok(()) => Ok(()),
            Err(error) if error.started => Ok(()),
            Err(error) => Err(match error.error {
                crate::contained_model::ModelError::Request => EgressError::Request,
                crate::contained_model::ModelError::Revoked => EgressError::Authentication,
                _ => EgressError::Transport,
            }),
        };
    }
    let upstream = policy.forward(request)?;
    let _ = wire::response(upstream, stream);
    Ok(())
}
