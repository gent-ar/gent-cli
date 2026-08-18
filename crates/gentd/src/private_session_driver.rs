//! Private, provider-neutral persist-before-publication session supervision.
//!
//! This is not a daemon service or transport. A future authority composition owns one driver per
//! already-approved session and supplies a durable port. No client, bootstrap path, or capability
//! can reach it until that composition is separately proven.

use std::collections::{BTreeMap, VecDeque};

use gent_runtime::ProviderLifecycleEffect;
use gent_types::HostEpoch;

const MAX_PENDING: usize = 16;
const MAX_RETAINED_DELTAS: usize = 32;

/// A provider adapter's already-normalized input. Raw stdout and provider sessions are excluded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionIngress {
    pub(crate) source_id: String,
    pub(crate) effect: ProviderLifecycleEffect,
}

impl PrivateSessionIngress {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    const fn is_terminal(&self) -> bool {
        matches!(self.effect, ProviderLifecycleEffect::Terminal { .. })
    }
}

/// A durable projection supplied only after an ingress fact has been committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionPersisted<D> {
    pub(crate) cursor: u64,
    pub(crate) delta: D,
    pub(crate) terminal: bool,
}

/// An authoritative reconnect projection supplied by a daemon-owned durable port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionSnapshot<S> {
    pub(crate) host_epoch: HostEpoch,
    pub(crate) cursor: u64,
    pub(crate) terminal: bool,
    pub(crate) value: S,
}

/// Durable port for one scoped session. `persist` must settle a terminal input atomically.
pub(crate) trait PrivateSessionPort {
    type Delta: Clone + Eq;
    type Snapshot: Clone + Eq;
    type Error;

    fn persist(
        &mut self,
        ingress: &PrivateSessionIngress,
    ) -> Result<PrivateSessionPersisted<Self::Delta>, Self::Error>;
    fn snapshot(&self) -> Result<PrivateSessionSnapshot<Self::Snapshot>, Self::Error>;
}

/// A delta emitted only after the durable port has accepted its corresponding normalized fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateSessionDelta<D> {
    pub(crate) cursor: u64,
    pub(crate) value: D,
}

/// Result of a reconnect request. A client replaces local state on [`Self::Resync`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateSessionResume<S, D> {
    Delta {
        deltas: Vec<PrivateSessionDelta<D>>,
        terminal: bool,
    },
    Resync(PrivateSessionSnapshot<S>),
}

/// A single bounded session-driver tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateSessionDrive {
    Idle,
    Persisted { cursor: u64, terminal: bool },
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
    HostEpochChanged,
    CursorDidNotAdvance,
    TerminalMismatch,
}

type PrivateSessionResumeResult<P> = Result<
    PrivateSessionResume<<P as PrivateSessionPort>::Snapshot, <P as PrivateSessionPort>::Delta>,
    PrivateSessionError<<P as PrivateSessionPort>::Error>,
>;

/// Bounded, private session supervisor with a durable snapshot/resume fence.
#[derive(Debug)]
pub(crate) struct PrivateSessionDriver<P>
where
    P: PrivateSessionPort,
{
    port: P,
    host_epoch: HostEpoch,
    cursor: u64,
    resume_floor: u64,
    terminal: bool,
    pending: VecDeque<PrivateSessionIngress>,
    persisted: BTreeMap<String, (PrivateSessionIngress, u64)>,
    deltas: VecDeque<PrivateSessionDelta<P::Delta>>,
}

impl<P> PrivateSessionDriver<P>
where
    P: PrivateSessionPort,
{
    /// Opens exactly one session against a snapshot from its expected daemon host epoch.
    pub(crate) fn open(
        port: P,
        host_epoch: HostEpoch,
    ) -> Result<Self, PrivateSessionError<P::Error>> {
        let snapshot = port.snapshot().map_err(PrivateSessionError::Port)?;
        if snapshot.host_epoch != host_epoch {
            return Err(PrivateSessionError::HostEpochChanged);
        }
        Ok(Self {
            port,
            host_epoch,
            cursor: snapshot.cursor,
            resume_floor: snapshot.cursor,
            terminal: snapshot.terminal,
            pending: VecDeque::new(),
            persisted: BTreeMap::new(),
            deltas: VecDeque::new(),
        })
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

    /// Persists and exposes at most one queued normalized fact on the caller's tick.
    pub(crate) fn drive(&mut self) -> Result<PrivateSessionDrive, PrivateSessionError<P::Error>> {
        let Some(ingress) = self.pending.front().cloned() else {
            return Ok(PrivateSessionDrive::Idle);
        };
        let persisted = self
            .port
            .persist(&ingress)
            .map_err(PrivateSessionError::Port)?;
        if persisted.cursor <= self.cursor {
            return Err(PrivateSessionError::CursorDidNotAdvance);
        }
        if persisted.terminal != ingress.is_terminal() {
            return Err(PrivateSessionError::TerminalMismatch);
        }
        self.pending.pop_front();
        self.cursor = persisted.cursor;
        self.terminal = persisted.terminal;
        self.persisted
            .insert(ingress.source_id().into(), (ingress, persisted.cursor));
        self.deltas.push_back(PrivateSessionDelta {
            cursor: persisted.cursor,
            value: persisted.delta,
        });
        if self.deltas.len() > MAX_RETAINED_DELTAS {
            self.resume_floor = self
                .deltas
                .pop_front()
                .expect("retained delta exists")
                .cursor;
        }
        Ok(PrivateSessionDrive::Persisted {
            cursor: persisted.cursor,
            terminal: self.terminal,
        })
    }

    /// Returns retained ordered deltas or forces the caller to replace local state from snapshot.
    pub(crate) fn resume(&self, after_cursor: u64) -> PrivateSessionResumeResult<P> {
        if after_cursor < self.resume_floor || after_cursor > self.cursor {
            return self.snapshot().map(PrivateSessionResume::Resync);
        }
        Ok(PrivateSessionResume::Delta {
            deltas: self
                .deltas
                .iter()
                .filter(|delta| delta.cursor > after_cursor)
                .cloned()
                .collect(),
            terminal: self.terminal,
        })
    }

    fn snapshot(
        &self,
    ) -> Result<PrivateSessionSnapshot<P::Snapshot>, PrivateSessionError<P::Error>> {
        let snapshot = self.port.snapshot().map_err(PrivateSessionError::Port)?;
        if snapshot.host_epoch != self.host_epoch {
            return Err(PrivateSessionError::HostEpochChanged);
        }
        if snapshot.cursor < self.cursor || (self.terminal && !snapshot.terminal) {
            return Err(PrivateSessionError::CursorDidNotAdvance);
        }
        Ok(snapshot)
    }
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
