//! Private bridge composition for one Gent-owned upstream `claurst acp` connection.
//!
//! The bridge accepts only durable Gent prompt/context identities, owns all ACP session IDs, and
//! returns cursor-sealed normalized facts to `PrivateClaurstIngress`.  It deliberately has no
//! daemon bootstrap, IPC, model selection, or provider configuration surface.

use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

use async_trait::async_trait;
use gent_drivers::conversation_context_input::{
    MAX_FRESH_CONTEXT_INPUT_BYTES, render_fresh_conversation_input,
};
use gent_ports::{
    ClaurstCheckpoint, ClaurstDrainBatch, ClaurstDrainRequest, ClaurstFactValue,
    ClaurstNormalizedFact, ClaurstSessionBinding, ClaurstSourceId, ClaurstStartRequest,
    ClaurstSubmitRequest, ClaurstTerminal, PortError, PrivateClaurstBridge,
};
use sha2::{Digest, Sha256};

use crate::claurst_acp_transport::{
    ClaurstAcpFact, ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport,
    ClaurstAcpTransportError,
};

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
    state: Mutex<BridgeState<S>>,
}

impl<S: ClaurstAcpStdio> ClaurstAcpBridge<S> {
    /// Creates a bridge rooted in the absolute Gent workspace supplied to ACP `session/new`.
    #[must_use]
    pub(crate) fn new(workspace: PathBuf, stdio: S) -> Self {
        Self {
            workspace,
            state: Mutex::new(BridgeState {
                transport: ClaurstAcpTransport::new(stdio),
                sources: BTreeMap::new(),
            }),
        }
    }
}

#[async_trait]
impl<S> PrivateClaurstBridge for ClaurstAcpBridge<S>
where
    S: ClaurstAcpStdio + Send,
{
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        request.validate().map_err(|_| invalid("start request"))?;
        let input = render_fresh_conversation_input(
            &request.context,
            &request.prompt,
            MAX_FRESH_CONTEXT_INPUT_BYTES,
        )
        .map_err(|_| invalid("frozen conversation context"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| unavailable("ACP bridge lock"))?;
        if state.sources.contains_key(&request.source_id) {
            return Err(invalid("duplicate source"));
        }
        let session_id = state
            .transport
            .initialize_session(&self.workspace)
            .map_err(provider)?;
        state
            .transport
            .prompt(&session_id, input.prompt())
            .map_err(provider)?;
        let binding = ClaurstSessionBinding {
            run_id: request.run_id,
            source_id: request.source_id,
            opaque_session_id: session_id,
        };
        state.sources.insert(
            binding.source_id.clone(),
            SourceState {
                binding: binding.clone(),
                cursor: 0,
                terminal: false,
            },
        );
        Ok(binding)
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        let state = self
            .state
            .lock()
            .map_err(|_| unavailable("ACP bridge lock"))?;
        (state
            .sources
            .get(&binding.source_id)
            .is_some_and(|source| source.binding == binding))
        .then_some(())
        .ok_or_else(|| invalid("unknown session binding"))
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        request.validate().map_err(|_| invalid("submit request"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| unavailable("ACP bridge lock"))?;
        let source = state
            .sources
            .get(&request.binding.source_id)
            .ok_or_else(|| invalid("unknown source"))?;
        if source.binding != request.binding || source.terminal {
            return Err(invalid("inactive session binding"));
        }
        state
            .transport
            .prompt(&request.binding.opaque_session_id, &request.prompt)
            .map_err(provider)
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        if !request.is_bounded() {
            return Err(invalid("unbounded drain"));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| unavailable("ACP bridge lock"))?;
        let source = state
            .sources
            .get(&request.source_id)
            .ok_or_else(|| invalid("unknown source"))?;
        if source.binding.run_id != request.run_id
            || source.cursor != request.after_cursor
            || source.terminal
        {
            return Err(invalid("stale or terminal drain"));
        }
        let acp = state.transport.drain(request.limit).map_err(provider)?;
        let source = state
            .sources
            .get_mut(&request.source_id)
            .expect("source remains while bridge lock is held");
        let facts = acp
            .facts
            .into_iter()
            .map(|fact| {
                source.cursor += 1;
                ClaurstNormalizedFact {
                    source_id: request.source_id.clone(),
                    cursor: source.cursor,
                    value: project(fact),
                }
            })
            .collect();
        let terminal = acp.terminal.map(project_terminal);
        source.terminal = terminal.is_some();
        Ok(ClaurstDrainBatch {
            facts,
            checkpoint: Some(checkpoint(&source.binding, source.cursor)),
            session_binding: Some(source.binding.clone()),
            terminal,
        })
    }
}

fn project(fact: ClaurstAcpFact) -> ClaurstFactValue {
    match fact {
        ClaurstAcpFact::Event(event) => ClaurstFactValue::Event(event),
        ClaurstAcpFact::Lifecycle(signal) => ClaurstFactValue::Lifecycle(signal),
    }
}

fn project_terminal(terminal: ClaurstAcpTerminal) -> ClaurstTerminal {
    match terminal {
        ClaurstAcpTerminal::Completed => ClaurstTerminal::Completed,
        ClaurstAcpTerminal::Interrupted => ClaurstTerminal::Interrupted,
        ClaurstAcpTerminal::Failed => ClaurstTerminal::Failed {
            classification: gent_ports::ClaurstFailureClassification::Protocol,
        },
    }
}

fn checkpoint(binding: &ClaurstSessionBinding, cursor: u64) -> ClaurstCheckpoint {
    let digest = format!(
        "{}\0{}\0{}\0{cursor}",
        binding.run_id, binding.source_id.0, binding.opaque_session_id
    );
    ClaurstCheckpoint {
        run_id: binding.run_id.clone(),
        source_id: binding.source_id.clone(),
        cursor,
        state_digest_sha256: format!("{:x}", Sha256::digest(digest.as_bytes())),
    }
}

fn provider(error: ClaurstAcpTransportError) -> PortError {
    PortError::Provider(error.to_string())
}

fn invalid(what: &str) -> PortError {
    PortError::Provider(format!("invalid Claurst ACP {what}"))
}

fn unavailable(what: &str) -> PortError {
    PortError::Unavailable(what.into())
}

#[cfg(test)]
#[path = "claurst_acp_bridge_tests.rs"]
mod tests;
