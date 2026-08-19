//! Observer-gated runtime access to typed, daemon-owned orchestration graphs.

use gent_ports::{OrchestrationLedger, OrchestrationWrite};
use gent_types::{CrossReviewRequest, FanoutRequest, TaskGraph, TaskGraphFactPage};

use crate::RuntimeError;

/// Explicit composition authority for planned multi-agent orchestration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OrchestrationAuthority {
    #[default]
    Observer,
    Approved,
}

/// Closed result of one authority-gated graph operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationResult {
    DeniedObserver,
    Missing,
    Graph(Box<TaskGraph>),
    FactPage(TaskGraphFactPage),
}

/// Coordinates typed graph requests with an injected durable ledger and no provider access.
#[derive(Clone, Debug)]
pub struct OrchestrationService<L> {
    ledger: L,
    authority: OrchestrationAuthority,
}

impl<L> OrchestrationService<L> {
    /// Creates an inert service unless a future evidence-approved composition enables it.
    #[must_use]
    pub fn new(ledger: L, authority: OrchestrationAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: OrchestrationLedger> OrchestrationService<L> {
    /// Reads one graph only through approved composition.
    ///
    /// # Errors
    /// Returns a durable read error only after approved composition reaches the ledger.
    pub fn graph(&self, graph_id: &str) -> Result<OrchestrationResult, RuntimeError> {
        if self.authority != OrchestrationAuthority::Approved {
            return Ok(OrchestrationResult::DeniedObserver);
        }
        Ok(self
            .ledger
            .task_graph(graph_id)?
            .map_or(OrchestrationResult::Missing, |graph| {
                OrchestrationResult::Graph(Box::new(graph))
            }))
    }
    /// Reads a bounded ordered page of canonical graph facts through approved composition.
    ///
    /// # Errors
    /// Returns a durable read error only after approved composition reaches the ledger.
    pub fn graph_facts(
        &self,
        graph_id: &str,
        after_cursor: u64,
        limit: u16,
    ) -> Result<OrchestrationResult, RuntimeError> {
        if self.authority != OrchestrationAuthority::Approved {
            return Ok(OrchestrationResult::DeniedObserver);
        }
        self.ledger
            .task_graph_facts(graph_id, after_cursor, limit)
            .map(OrchestrationResult::FactPage)
            .map_err(RuntimeError::from)
    }
    /// Applies a typed fanout intent only through approved composition.
    ///
    /// # Errors
    /// Returns a durable fence or write error only after approved composition reaches the ledger.
    pub fn fanout(&self, request: &FanoutRequest) -> Result<OrchestrationResult, RuntimeError> {
        if self.authority != OrchestrationAuthority::Approved {
            return Ok(OrchestrationResult::DeniedObserver);
        }
        self.ledger
            .apply_fanout(request)
            .map(map)
            .map_err(RuntimeError::from)
    }
    /// Applies a typed cross-review intent only through approved composition.
    ///
    /// # Errors
    /// Returns a durable fence or write error only after approved composition reaches the ledger.
    pub fn cross_review(
        &self,
        request: &CrossReviewRequest,
    ) -> Result<OrchestrationResult, RuntimeError> {
        if self.authority != OrchestrationAuthority::Approved {
            return Ok(OrchestrationResult::DeniedObserver);
        }
        self.ledger
            .apply_cross_review(request)
            .map(map)
            .map_err(RuntimeError::from)
    }
}

fn map(write: OrchestrationWrite) -> OrchestrationResult {
    match write {
        OrchestrationWrite::Created(graph)
        | OrchestrationWrite::Updated(graph)
        | OrchestrationWrite::Current(graph) => OrchestrationResult::Graph(Box::new(graph)),
    }
}
