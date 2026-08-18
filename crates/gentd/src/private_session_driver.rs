//! Private, provider-neutral persist-before-publication session supervision.
//!
//! This is not a daemon service or transport. A future authority composition owns one driver per
//! already-approved session and supplies a durable port. No client, bootstrap path, or capability
//! can reach it until that composition is separately proven.

use std::collections::{BTreeMap, VecDeque};

use crate::private_session_atomic_port::{PrivateSessionAtomicBatch, PrivateSessionAtomicPort};
use gent_runtime::ProviderLifecycleEffect;

const MAX_PENDING: usize = 16;
const MAX_RETAINED_DELTAS: usize = 32;

/// A provider adapter's already-normalized input. Raw stdout and provider sessions are excluded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionIngress {
    pub(crate) source_id: String,
    pub(crate) effect: ProviderLifecycleEffect,
}

impl PrivateSessionIngress {
    pub(crate) fn source_id(&self) -> &str {
        &self.source_id
    }

    pub(crate) const fn is_terminal(&self) -> bool {
        matches!(self.effect, ProviderLifecycleEffect::Terminal { .. })
    }
}

/// A delta emitted only after the durable port has accepted its corresponding normalized fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionDelta<D> {
    pub(crate) cursor: u64,
    pub(crate) value: D,
}

/// Result of a reconnect request. Durable replay is requested when the local buffer is insufficient.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateSessionResume<D> {
    Delta {
        deltas: Vec<PrivateSessionDelta<D>>,
        terminal: bool,
    },
    ReplayRequired {
        after_cursor: u64,
        through_cursor: u64,
        terminal: bool,
    },
}

/// A single bounded session-driver tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateSessionDrive {
    Idle,
    Persisted { cursors: Vec<u64>, terminal: bool },
}

/// Successful enqueue outcome; a duplicate is never persisted or rebroadcast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateSessionEnqueue {
    Queued,
    AlreadyPersisted { cursor: u64 },
}

/// Input failures are rejected before the durable port is called.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateSessionEnqueueError {
    EmptySourceId,
    EmptyTerminalReason,
    Backpressured,
    SourceIdCollision,
    TerminalQueued,
    TerminalSettled,
}

/// A driver error leaves queued input intact so its caller controls retry timing.
#[derive(Debug)]
pub(crate) enum PrivateSessionError<E> {
    Port(E),
    CursorDidNotAdvance,
    BatchShape,
    SourceIdMismatch,
    TerminalMismatch,
}

/// Bounded, private session supervisor with cursor-only durable replay fencing.
#[derive(Debug)]
pub(crate) struct PrivateSessionDriver<P>
where
    P: PrivateSessionAtomicPort,
{
    port: P,
    cursor: u64,
    resume_floor: u64,
    terminal: bool,
    pending: VecDeque<PrivateSessionIngress>,
    persisted: BTreeMap<String, (PrivateSessionIngress, u64)>,
    deltas: VecDeque<PrivateSessionDelta<P::Delta>>,
}

impl<P> PrivateSessionDriver<P>
where
    P: PrivateSessionAtomicPort,
{
    /// Opens exactly one empty in-memory replay window. Earlier deltas remain in the ledger.
    pub(crate) fn open(port: P) -> Self {
        Self {
            port,
            cursor: 0,
            resume_floor: 0,
            terminal: false,
            pending: VecDeque::new(),
            persisted: BTreeMap::new(),
            deltas: VecDeque::new(),
        }
    }

    /// Queues one normalized fact without performing persistence or publication.
    pub(crate) fn enqueue(
        &mut self,
        ingress: PrivateSessionIngress,
    ) -> Result<PrivateSessionEnqueue, PrivateSessionEnqueueError> {
        validate(&ingress)?;
        if let Some((known, cursor)) = self.persisted.get(ingress.source_id()) {
            return (known == &ingress)
                .then_some(PrivateSessionEnqueue::AlreadyPersisted { cursor: *cursor })
                .ok_or(PrivateSessionEnqueueError::SourceIdCollision);
        }
        if self
            .pending
            .iter()
            .any(|known| known.source_id() == ingress.source_id())
        {
            return self
                .pending
                .iter()
                .any(|known| known == &ingress)
                .then_some(PrivateSessionEnqueue::Queued)
                .ok_or(PrivateSessionEnqueueError::SourceIdCollision);
        }
        if self.terminal {
            return Err(PrivateSessionEnqueueError::TerminalSettled);
        }
        if self.pending.iter().any(PrivateSessionIngress::is_terminal) {
            return Err(PrivateSessionEnqueueError::TerminalQueued);
        }
        if self.pending.len() == MAX_PENDING {
            return Err(PrivateSessionEnqueueError::Backpressured);
        }
        self.pending.push_back(ingress);
        Ok(PrivateSessionEnqueue::Queued)
    }

    /// Persists and exposes one bounded all-or-nothing normalized batch on the caller's tick.
    pub(crate) fn drive(&mut self) -> Result<PrivateSessionDrive, PrivateSessionError<P::Error>> {
        if self.pending.is_empty() {
            return Ok(PrivateSessionDrive::Idle);
        }
        let ingress = self.pending.iter().cloned().collect::<Vec<_>>();
        let persisted = self
            .port
            .persist_atomic_batch(&ingress)
            .map_err(PrivateSessionError::Port)?;
        validate_batch(&ingress, &persisted, self.cursor)?;
        let cursors = persisted
            .records
            .iter()
            .map(|record| record.cursor)
            .collect();
        self.apply_batch(ingress, persisted);
        Ok(PrivateSessionDrive::Persisted {
            cursors,
            terminal: self.terminal,
        })
    }

    /// Returns retained ordered deltas or requests a durable cursor replay.
    pub(crate) fn resume(&self, after_cursor: u64) -> PrivateSessionResume<P::Delta> {
        if after_cursor < self.resume_floor || after_cursor > self.cursor {
            return PrivateSessionResume::ReplayRequired {
                after_cursor,
                through_cursor: self.cursor,
                terminal: self.terminal,
            };
        }
        PrivateSessionResume::Delta {
            deltas: self
                .deltas
                .iter()
                .filter(|delta| delta.cursor > after_cursor)
                .cloned()
                .collect(),
            terminal: self.terminal,
        }
    }

    fn apply_batch(
        &mut self,
        ingress: Vec<PrivateSessionIngress>,
        persisted: PrivateSessionAtomicBatch<P::Delta>,
    ) {
        for (ingress, record) in ingress.into_iter().zip(persisted.records) {
            self.pending.pop_front();
            self.cursor = record.cursor;
            self.terminal = record.terminal;
            self.persisted
                .insert(ingress.source_id().into(), (ingress, record.cursor));
            self.deltas.push_back(PrivateSessionDelta {
                cursor: record.cursor,
                value: record.delta,
            });
        }
        while self.deltas.len() > MAX_RETAINED_DELTAS {
            self.resume_floor = self
                .deltas
                .pop_front()
                .expect("retained delta exists")
                .cursor;
        }
    }
}

fn validate_batch<D, E>(
    ingress: &[PrivateSessionIngress],
    persisted: &PrivateSessionAtomicBatch<D>,
    previous_cursor: u64,
) -> Result<(), PrivateSessionError<E>> {
    if persisted.records.len() != ingress.len() || persisted.records.is_empty() {
        return Err(PrivateSessionError::BatchShape);
    }
    let mut cursor = previous_cursor;
    for (index, (ingress, record)) in ingress.iter().zip(&persisted.records).enumerate() {
        if record.source_id != ingress.source_id() {
            return Err(PrivateSessionError::SourceIdMismatch);
        }
        if record.cursor <= cursor {
            return Err(PrivateSessionError::CursorDidNotAdvance);
        }
        if record.terminal != ingress.is_terminal()
            || (record.terminal && index + 1 != persisted.records.len())
        {
            return Err(PrivateSessionError::TerminalMismatch);
        }
        cursor = record.cursor;
    }
    Ok(())
}

fn validate(ingress: &PrivateSessionIngress) -> Result<(), PrivateSessionEnqueueError> {
    if ingress.source_id().trim().is_empty() {
        return Err(PrivateSessionEnqueueError::EmptySourceId);
    }
    if matches!(&ingress.effect, ProviderLifecycleEffect::Terminal { reason } if reason.trim().is_empty())
    {
        return Err(PrivateSessionEnqueueError::EmptyTerminalReason);
    }
    Ok(())
}

#[cfg(test)]
#[path = "private_session_driver_tests.rs"]
mod tests;
