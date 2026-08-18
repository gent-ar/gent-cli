//! Scriptable fake for the private Claurst normalized-fact boundary.
#![allow(clippy::missing_panics_doc)] // Test fakes fail fast on poisoned state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use gent_ports::{
    ClaurstDrainBatch, ClaurstDrainRequest, ClaurstNormalizedFact, ClaurstSessionBinding,
    ClaurstSourceId, ClaurstStartRequest, ClaurstSubmitRequest, PortError, PrivateClaurstBridge,
};

#[derive(Debug, Default)]
struct BridgeState {
    bindings: BTreeMap<ClaurstSourceId, ClaurstSessionBinding>,
    starts: Vec<ClaurstStartRequest>,
    submissions: Vec<ClaurstSubmitRequest>,
    start_bindings: VecDeque<Result<ClaurstSessionBinding, String>>,
    requests: Vec<ClaurstDrainRequest>,
    batches: VecDeque<Result<ClaurstDrainBatch, String>>,
    settled: BTreeSet<ClaurstSourceId>,
}

/// Deterministic fake that enforces the private bridge's ordering and secrecy contract.
#[derive(Debug, Default)]
pub struct FakePrivateClaurstBridge {
    state: Mutex<BridgeState>,
}

impl FakePrivateClaurstBridge {
    /// Queues the private session binding returned for the next valid daemon-owned start.
    pub fn push_start_binding(&self, binding: ClaurstSessionBinding) {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .start_bindings
            .push_back(Ok(binding));
    }

    /// Queues one result for the next valid drain request.
    pub fn push_batch(&self, batch: ClaurstDrainBatch) {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .batches
            .push_back(Ok(batch));
    }

    /// Queues one controlled bridge failure for the next valid drain request.
    pub fn fail_next_drain(&self, message: impl Into<String>) {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .batches
            .push_back(Err(message.into()));
    }

    /// Returns every daemon-owned session binding the fake observed.
    #[must_use]
    pub fn bindings(&self) -> Vec<ClaurstSessionBinding> {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .bindings
            .values()
            .cloned()
            .collect()
    }

    /// Returns normalized start input the daemon supplied, never provider configuration.
    #[must_use]
    pub fn starts(&self) -> Vec<ClaurstStartRequest> {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .starts
            .clone()
    }

    /// Returns follow-up inputs accepted for an already bound private source.
    #[must_use]
    pub fn submissions(&self) -> Vec<ClaurstSubmitRequest> {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .submissions
            .clone()
    }

    /// Returns drain requests in the order a daemon would have issued them.
    #[must_use]
    pub fn requests(&self) -> Vec<ClaurstDrainRequest> {
        self.state
            .lock()
            .expect("private bridge fake mutex poisoned")
            .requests
            .clone()
    }
}

#[async_trait]
impl PrivateClaurstBridge for FakePrivateClaurstBridge {
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        request
            .validate()
            .map_err(|_| contract_error("start input is invalid"))?;
        let mut state = self
            .state
            .lock()
            .expect("private bridge fake mutex poisoned");
        state.starts.push(request.clone());
        let binding = state
            .start_bindings
            .pop_front()
            .ok_or_else(|| contract_error("no queued start binding"))?
            .map_err(PortError::Provider)?;
        (binding.run_id == request.run_id && binding.source_id == request.source_id)
            .then_some(binding)
            .ok_or_else(|| contract_error("start binding does not match request"))
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        if binding.run_id.is_empty()
            || binding.source_id.0.is_empty()
            || binding.opaque_session_id.is_empty()
        {
            return Err(contract_error("session binding is incomplete"));
        }
        let mut state = self
            .state
            .lock()
            .expect("private bridge fake mutex poisoned");
        match state.bindings.get(&binding.source_id) {
            Some(existing) if existing != &binding => {
                Err(contract_error("source is already bound to another session"))
            }
            _ => {
                state.bindings.insert(binding.source_id.clone(), binding);
                Ok(())
            }
        }
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        request
            .validate()
            .map_err(|_| contract_error("submit input is invalid"))?;
        let mut state = self
            .state
            .lock()
            .expect("private bridge fake mutex poisoned");
        (state.bindings.get(&request.binding.source_id) == Some(&request.binding))
            .then_some(())
            .ok_or_else(|| contract_error("submit source has no matching session"))?;
        state.submissions.push(request);
        Ok(())
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        if !request.is_bounded() {
            return Err(contract_error("drain exceeds bounded contract"));
        }
        let mut state = self
            .state
            .lock()
            .expect("private bridge fake mutex poisoned");
        let binding = state
            .bindings
            .get(&request.source_id)
            .cloned()
            .ok_or_else(|| contract_error("source has no daemon session binding"))?;
        if binding.run_id != request.run_id || state.settled.contains(&request.source_id) {
            return Err(contract_error("source cannot be drained for this run"));
        }
        state.requests.push(request.clone());
        let batch = state
            .batches
            .pop_front()
            .transpose()
            .map_err(PortError::Provider)?;
        let Some(batch) = batch else {
            return Ok(ClaurstDrainBatch {
                facts: Vec::new(),
                checkpoint: None,
                session_binding: None,
                terminal: None,
            });
        };
        validate_batch(&request, &binding, &batch)?;
        if batch.terminal.is_some() {
            state.settled.insert(request.source_id);
        }
        Ok(batch)
    }
}

fn validate_batch(
    request: &ClaurstDrainRequest,
    binding: &ClaurstSessionBinding,
    batch: &ClaurstDrainBatch,
) -> Result<(), PortError> {
    if batch.facts.len() > usize::from(request.limit)
        || batch
            .facts
            .iter()
            .any(|fact| invalid_fact(request, binding, fact))
        || !strictly_increasing(&batch.facts)
    {
        return Err(contract_error(
            "batch violates ordered bounded fact contract",
        ));
    }
    if let Some(checkpoint) = &batch.checkpoint {
        if checkpoint.run_id != request.run_id
            || checkpoint.source_id != request.source_id
            || checkpoint.cursor < request.after_cursor
            || checkpoint.cursor
                < batch
                    .facts
                    .last()
                    .map_or(request.after_cursor, |fact| fact.cursor)
            || !valid_digest(&checkpoint.state_digest_sha256)
            || checkpoint
                .state_digest_sha256
                .contains(&binding.opaque_session_id)
        {
            return Err(contract_error("checkpoint does not bind this drain"));
        }
    }
    if let Some(session) = &batch.session_binding {
        if session.run_id != request.run_id
            || session.source_id != request.source_id
            || session.opaque_session_id.is_empty()
            || facts_echo_session(&batch.facts, &session.opaque_session_id)
        {
            return Err(contract_error("batch would echo an opaque session"));
        }
    }
    if facts_echo_session(&batch.facts, &binding.opaque_session_id) {
        return Err(contract_error("batch would echo an opaque session"));
    }
    Ok(())
}

fn strictly_increasing(facts: &[ClaurstNormalizedFact]) -> bool {
    facts.windows(2).all(|pair| pair[0].cursor < pair[1].cursor)
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_fact(
    request: &ClaurstDrainRequest,
    binding: &ClaurstSessionBinding,
    fact: &ClaurstNormalizedFact,
) -> bool {
    fact.source_id != request.source_id
        || fact.cursor <= request.after_cursor
        || fact.cursor == 0
        || facts_echo_session(std::slice::from_ref(fact), &binding.opaque_session_id)
}

fn facts_echo_session(facts: &[ClaurstNormalizedFact], opaque_session: &str) -> bool {
    facts
        .iter()
        .any(|fact| format!("{:?}", fact.value).contains(opaque_session))
}

fn contract_error(message: &str) -> PortError {
    PortError::Provider(format!("private Claurst bridge contract: {message}"))
}
