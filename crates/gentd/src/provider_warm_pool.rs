use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use gent_protocol::ProviderWarmOutcome;

use crate::provider_run_state::RunLiveness;

const UNPROMPTED_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const SERVED_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const FAILED_WARM_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub(crate) struct WarmFailures(BTreeMap<String, Instant>);

impl WarmFailures {
    pub(crate) fn record(&mut self, run_id: &str, now: Instant) {
        self.0.insert(run_id.to_owned(), now);
    }

    pub(crate) fn backing_off(&mut self, run_id: &str, now: Instant) -> bool {
        self.0
            .retain(|_, failed| now.saturating_duration_since(*failed) < FAILED_WARM_BACKOFF);
        self.0.contains_key(run_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WarmTarget {
    pub(crate) conversation_id: String,
    pub(crate) run_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WarmPoolRequest {
    pub(crate) now: Instant,
    pub(crate) target: Option<WarmTarget>,
}

impl WarmPoolRequest {
    pub(crate) fn unavailable(&self) -> Option<ProviderWarmOutcome> {
        self.refused(ProviderWarmOutcome::Unavailable)
    }

    pub(crate) fn refused(&self, outcome: ProviderWarmOutcome) -> Option<ProviderWarmOutcome> {
        self.target.as_ref().map(|_| outcome)
    }
}

pub(crate) struct WarmRun<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) conversation_id: &'a str,
    pub(crate) idle: bool,
    pub(crate) last_dispatch: u64,
}

pub(crate) struct IdleRun<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) idle_for: Duration,
    pub(crate) served: bool,
    pub(crate) current: bool,
}

pub(crate) struct IdleCandidate {
    run_id: String,
    conversation_id: String,
    since: Instant,
    served: bool,
}

impl IdleCandidate {
    pub(crate) fn served(
        run_id: &str,
        conversation_id: &str,
        idle_since: &mut Option<Instant>,
        liveness: RunLiveness,
        now: Instant,
    ) -> Option<Self> {
        if !liveness.idle() {
            *idle_since = None;
            return None;
        }
        Some(Self {
            run_id: run_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            since: *idle_since.get_or_insert(now),
            served: true,
        })
    }

    pub(crate) fn unprompted(run_id: &str, conversation_id: &str, since: Instant) -> Self {
        Self {
            run_id: run_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            since,
            served: false,
        }
    }
}

pub(crate) fn reaped<E>(
    now: Instant,
    candidates: &[IdleCandidate],
    is_current: impl Fn(&str, &str) -> Result<bool, E>,
) -> Vec<String> {
    expired(candidates.iter().map(|candidate| IdleRun {
        run_id: &candidate.run_id,
        idle_for: now.saturating_duration_since(candidate.since),
        served: candidate.served,
        current: !matches!(
            is_current(&candidate.conversation_id, &candidate.run_id),
            Ok(false)
        ),
    }))
}

struct Room {
    evicted: Vec<String>,
    fits: bool,
}

pub(crate) fn evictions<'a>(
    runs: impl IntoIterator<Item = WarmRun<'a>>,
    run_id: &str,
    conversation_id: &str,
    capacity: usize,
) -> Vec<String> {
    room(runs, run_id, conversation_id, capacity).evicted
}

pub(crate) fn admission<'a>(
    runs: impl IntoIterator<Item = WarmRun<'a>>,
    run_id: &str,
    conversation_id: &str,
    capacity: usize,
) -> Result<Vec<String>, ProviderWarmOutcome> {
    let runs = runs.into_iter().collect::<Vec<_>>();
    if let Some(run) = runs.iter().find(|run| run.run_id == run_id) {
        return Err(if run.idle {
            ProviderWarmOutcome::AlreadyWarm
        } else {
            ProviderWarmOutcome::TurnActive
        });
    }
    if runs
        .iter()
        .any(|run| run.conversation_id == conversation_id && !run.idle)
    {
        return Err(ProviderWarmOutcome::TurnActive);
    }
    let room = room(runs, run_id, conversation_id, capacity);
    room.fits
        .then_some(room.evicted)
        .ok_or(ProviderWarmOutcome::PoolFull)
}

pub(crate) fn expired<'a>(runs: impl IntoIterator<Item = IdleRun<'a>>) -> Vec<String> {
    runs.into_iter()
        .filter(|run| {
            let timeout = if run.served {
                SERVED_IDLE_TIMEOUT
            } else {
                UNPROMPTED_IDLE_TIMEOUT
            };
            !run.current || run.idle_for >= timeout
        })
        .map(|run| run.run_id.to_owned())
        .collect()
}

pub(crate) fn release_each<E>(
    run_ids: Vec<String>,
    mut release: impl FnMut(&str) -> Result<(), E>,
) -> Result<(), E> {
    let mut failure = None;
    for run_id in run_ids {
        if let Err(error) = release(&run_id) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn room<'a>(
    runs: impl IntoIterator<Item = WarmRun<'a>>,
    run_id: &str,
    conversation_id: &str,
    capacity: usize,
) -> Room {
    let runs = runs.into_iter().collect::<Vec<_>>();
    let (superseded, kept): (Vec<_>, Vec<_>) = runs.into_iter().partition(|run| {
        run.idle && run.conversation_id == conversation_id && run.run_id != run_id
    });
    let mut idle = kept.iter().filter(|run| run.idle).collect::<Vec<_>>();
    idle.sort_by_key(|run| run.last_dispatch);
    let overflow = kept.len().saturating_add(1).saturating_sub(capacity.max(1));
    let reclaimed = overflow.min(idle.len());
    Room {
        evicted: superseded
            .iter()
            .chain(idle.into_iter().take(overflow))
            .map(|run| run.run_id.to_owned())
            .collect(),
        fits: reclaimed == overflow,
    }
}

#[cfg(test)]
#[path = "provider_warm_pool_tests.rs"]
mod tests;
