//! Provider-neutral atomic persistence boundary for one private session.
//!
//! The port receives only normalized ingress and returns one all-or-nothing durable batch. It is
//! deliberately unreachable from daemon bootstrap.

use crate::private_session_driver::PrivateSessionIngress;

/// One projection record produced by a committed atomic batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionAtomicRecord<D> {
    pub(crate) source_id: String,
    pub(crate) cursor: u64,
    pub(crate) delta: D,
    pub(crate) terminal: bool,
}

/// Complete result of one transaction; partial records are not a valid result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionAtomicBatch<D> {
    pub(crate) records: Vec<PrivateSessionAtomicRecord<D>>,
}

/// Durable adapter for normalized session batches.
pub(crate) trait PrivateSessionAtomicPort {
    type Delta: Clone + Eq;
    type Error;

    /// Commits every supplied normalized fact and its projection, or none of them.
    fn persist_atomic_batch(
        &mut self,
        ingress: &[PrivateSessionIngress],
    ) -> Result<PrivateSessionAtomicBatch<Self::Delta>, Self::Error>;
}
