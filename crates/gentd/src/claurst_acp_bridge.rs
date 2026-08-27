//! Private bridge composition for one Gent-owned upstream `claurst acp` connection.
//!
//! The bridge accepts only durable Gent prompt/context identities, owns all ACP session IDs, and
//! returns cursor-sealed normalized facts to `PrivateClaurstIngress`.  It deliberately has no
//! daemon bootstrap, IPC, model selection, or provider configuration surface.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use gent_ports::{
    ClaurstDrainBatch, ClaurstDrainRequest, ClaurstPermissionReply, ClaurstSessionBinding,
    ClaurstSourceId, ClaurstStartRequest, ClaurstSubmitRequest, PortError, PrivateClaurstBridge,
};

use crate::claurst_acp_transport::{ClaurstAcpStdio, ClaurstAcpTransport};

struct SourceState {
    binding: ClaurstSessionBinding,
    cursor: u64,
    terminal: bool,
}

struct BridgeState<S> {
    transport: ClaurstAcpTransport<S>,
    sources: BTreeMap<ClaurstSourceId, SourceState>,
}

/// A private ACP bridge that keeps upstream session identifiers inside the daemon.
pub(crate) struct ClaurstAcpBridge<S> {
    workspace: PathBuf,
    state: Arc<Mutex<BridgeState<S>>>,
}

impl<S: ClaurstAcpStdio> ClaurstAcpBridge<S> {
    /// Creates a bridge rooted in the absolute Gent workspace supplied to ACP `session/new`.
    #[must_use]
    pub(crate) fn new(workspace: PathBuf, stdio: S, mcp_servers: Vec<serde_json::Value>) -> Self {
        Self {
            workspace,
            state: Arc::new(Mutex::new(BridgeState {
                transport: ClaurstAcpTransport::new(stdio).with_mcp_servers(mcp_servers),
                sources: BTreeMap::new(),
            })),
        }
    }

    pub(crate) fn is_idle(&self) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "ACP bridge lock is unavailable".to_owned())?;
        Ok(state.transport.is_idle() && state.sources.values().all(|source| source.terminal))
    }

    pub(crate) async fn start_summary(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError>
    where
        S: ClaurstAcpStdio + Send + 'static,
    {
        let state = Arc::clone(&self.state);
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || start_summary_blocking(state, workspace, request))
            .await
            .map_err(|_| unavailable("ACP summary worker"))?
    }
}

#[async_trait]
impl<S> PrivateClaurstBridge for ClaurstAcpBridge<S>
where
    S: ClaurstAcpStdio + Send + 'static,
{
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        let state = Arc::clone(&self.state);
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || start_blocking(state, workspace, request))
            .await
            .map_err(|_| unavailable("ACP start worker"))?
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || bind_blocking(state, binding))
            .await
            .map_err(|_| unavailable("ACP bind worker"))?
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || submit_blocking(state, request))
            .await
            .map_err(|_| unavailable("ACP submit worker"))?
    }

    async fn cancel(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || cancel_blocking(state, binding))
            .await
            .map_err(|_| unavailable("ACP cancel worker"))?
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || drain_blocking(state, request))
            .await
            .map_err(|_| unavailable("ACP drain worker"))?
    }

    async fn respond_permission(
        &self,
        binding: ClaurstSessionBinding,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), PortError> {
        let state = Arc::clone(&self.state);
        let request_id = request_id.to_owned();
        tokio::task::spawn_blocking(move || {
            respond_permission_blocking(state, binding, &request_id, reply)
        })
        .await
        .map_err(|_| unavailable("ACP permission worker"))?
    }
}

pub(crate) struct ClaurstBridgeHandle<S>(Arc<ClaurstAcpBridge<S>>);

impl<S> Clone for ClaurstBridgeHandle<S> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<S> std::fmt::Debug for ClaurstBridgeHandle<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ClaurstBridgeHandle(..)")
    }
}

impl<S> ClaurstBridgeHandle<S> {
    #[must_use]
    pub(crate) fn new(bridge: Arc<ClaurstAcpBridge<S>>) -> Self {
        Self(bridge)
    }

    pub(crate) fn is_idle(&self) -> Result<bool, String>
    where
        S: ClaurstAcpStdio,
    {
        self.0.is_idle()
    }

    pub(crate) async fn start_summary(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError>
    where
        S: ClaurstAcpStdio + Send + 'static,
    {
        self.0.start_summary(request).await
    }
}

#[async_trait]
impl<S> PrivateClaurstBridge for ClaurstBridgeHandle<S>
where
    S: ClaurstAcpStdio + Send + 'static,
{
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        self.0.start(request).await
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0.bind_session(binding).await
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        self.0.submit(request).await
    }

    async fn cancel(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0.cancel(binding).await
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        self.0.drain(request).await
    }

    async fn respond_permission(
        &self,
        binding: ClaurstSessionBinding,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), PortError> {
        self.0.respond_permission(binding, request_id, reply).await
    }
}

#[path = "claurst_acp_bridge_support.rs"]
mod support;
use support::unavailable;
#[path = "claurst_acp_bridge_operations.rs"]
mod operations;
use operations::{
    bind_blocking, cancel_blocking, drain_blocking, respond_permission_blocking, start_blocking,
    start_summary_blocking, submit_blocking,
};

#[cfg(test)]
#[path = "claurst_acp_bridge_tests.rs"]
mod tests;
