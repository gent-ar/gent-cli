use std::collections::{BTreeMap, BTreeSet};

use gent_ports::LedgerError;
use gent_types::{
    ConversationAttentionStatus, ConversationErrorStatus, ConversationLiveStatus,
    ConversationProcessingStatus, ConversationWorkStatus, DurableTurnPhase, HostEpoch,
    NormalizedLifecycleSignal, NormalizedProviderEvent, NormalizedSessionLifecycle, RootActivity,
    RunLifecycleFact, RunLiveStatus, WorkPhase,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::queries::storage_error;

pub(super) const ROOT_FACTS: &str = "SELECT payload FROM run_lifecycle_facts WHERE run_id = ?1 AND coalesce(json_extract(payload, '$.lifecycle.event.type'), json_extract(payload, '$.lifecycle.signal.type')) IN ('turnStarted', 'turnEnded', 'rootActivity', 'providerFailure') ORDER BY cursor DESC";
pub(super) const WORK_FACTS: &str = "SELECT f.payload, json_extract(e.payload, '$.turnId') FROM run_lifecycle_facts f JOIN events e ON e.cursor = f.cursor WHERE f.run_id = ?1 AND coalesce(json_extract(f.payload, '$.lifecycle.event.type'), json_extract(f.payload, '$.lifecycle.signal.type')) IN ('childStarted', 'childPhase', 'childTerminal', 'commandPhase', 'commandTerminal') ORDER BY f.cursor ASC";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum WorkKind {
    Child,
    Command,
}

#[derive(Debug)]
struct Work {
    phase: WorkPhase,
    at: u64,
    turns: BTreeSet<String>,
}

type OpenWork = BTreeMap<(WorkKind, String), Work>;

#[derive(Debug, Default)]
struct RootState {
    activity: Option<RootActivity>,
    started_after: u64,
    failed: bool,
}

pub(super) fn read(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RunLiveStatus>, LedgerError> {
    let Some((cursor, host_epoch)) = connection
        .query_row(
            "SELECT cursor, host_epoch FROM run_lifecycle_facts WHERE run_id = ?1 ORDER BY cursor DESC LIMIT 1",
            [run_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
        )
        .optional()
        .map_err(storage_error)?
    else {
        return Ok(None);
    };
    let phase = latest_phase(connection, run_id)?;
    let live = phase.is_some_and(|phase| !phase.is_terminal());
    let root = if live {
        root(connection, run_id)?
    } else {
        RootState::default()
    };
    let generating = root.activity.is_some_and(RootActivity::is_generating);
    let work = work(connection, run_id)?;
    let failed_work = work
        .values()
        .any(|work| work.phase == WorkPhase::Failed && work.at > root.started_after);
    let waiting = matches!(
        phase,
        Some(DurableTurnPhase::WaitingPermission | DurableTurnPhase::WaitingQuestion)
    );
    let error = phase == Some(DurableTurnPhase::Failed) || (live && (root.failed || failed_work));
    Ok(Some(RunLiveStatus {
        run_id: run_id.into(),
        host_epoch: HostEpoch(host_epoch),
        status: ConversationLiveStatus {
            cursor,
            processing: if live {
                ConversationProcessingStatus::Processing
            } else {
                ConversationProcessingStatus::Idle
            },
            subagent_work: work_status(&work, WorkKind::Child, false, generating),
            command_work: work_status(
                &work,
                WorkKind::Command,
                super::background_tools::any_open(connection, run_id)?,
                generating,
            ),
            attention: if waiting || pending_permission(connection, run_id)? {
                ConversationAttentionStatus::Required
            } else {
                ConversationAttentionStatus::Clear
            },
            error: if error {
                ConversationErrorStatus::Error
            } else {
                ConversationErrorStatus::Clear
            },
        },
    }))
}

pub(super) fn latest_turn(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<(String, DurableTurnPhase)>, LedgerError> {
    connection
        .query_row(
            &format!(
                "SELECT t.turn_id, t.phase FROM turns t WHERE t.run_id = ?1 AND NOT {} ORDER BY t.sequence DESC LIMIT 1",
                super::agent_chat_queue_commands::turn_not_begun()
            ),
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    super::conversations::decode_phase(&row.get::<_, String>(1)?)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)
}

fn latest_phase(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<DurableTurnPhase>, LedgerError> {
    latest_turn(connection, run_id).map(|turn| turn.map(|(_, phase)| phase))
}

fn pending_permission(connection: &Connection, run_id: &str) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r JOIN pending_provider_permissions p ON p.conversation_id = r.conversation_id AND p.run_id = r.run_id WHERE r.run_id = ?1)",
            [run_id],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

fn root(connection: &Connection, run_id: &str) -> Result<RootState, LedgerError> {
    let mut statement = connection.prepare(ROOT_FACTS).map_err(storage_error)?;
    let mut rows = statement.query(params![run_id]).map_err(storage_error)?;
    let mut state = RootState::default();
    while let Some(row) = rows.next().map_err(storage_error)? {
        let fact = decode(&row.get::<_, String>(0).map_err(storage_error)?)?;
        match fact.lifecycle {
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::TurnStarted { .. },
            } => {
                state.activity.get_or_insert(RootActivity::Generating);
                state.started_after = fact.cursor;
                return Ok(state);
            }
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::TurnEnded { .. },
            } => {
                state.activity.get_or_insert(RootActivity::Idle);
            }
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::RootActivity { activity },
            }
            | NormalizedSessionLifecycle::Signal {
                signal: NormalizedLifecycleSignal::RootActivity { activity },
            } => {
                state.activity.get_or_insert(activity);
            }
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::ProviderFailure { .. },
            } => state.failed = true,
            _ => {}
        }
    }
    Ok(state)
}

pub(super) fn open_work_turns(
    connection: &Connection,
    run_id: &str,
) -> Result<BTreeSet<String>, LedgerError> {
    Ok(work(connection, run_id)?
        .into_values()
        .filter(|work| work.phase.is_live())
        .flat_map(|work| work.turns)
        .collect())
}

fn work(connection: &Connection, run_id: &str) -> Result<OpenWork, LedgerError> {
    let mut statement = connection.prepare(WORK_FACTS).map_err(storage_error)?;
    let mut rows = statement.query(params![run_id]).map_err(storage_error)?;
    let mut work = OpenWork::new();
    while let Some(row) = rows.next().map_err(storage_error)? {
        let fact = decode(&row.get::<_, String>(0).map_err(storage_error)?)?;
        let turn = row.get::<_, Option<String>>(1).map_err(storage_error)?;
        let entry = match fact.lifecycle {
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::ChildStarted { child_id, .. },
            } => Some(((WorkKind::Child, child_id), WorkPhase::Running)),
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::ChildTerminal { child_id, phase },
            }
            | NormalizedSessionLifecycle::Signal {
                signal: NormalizedLifecycleSignal::ChildPhase { child_id, phase },
            } => Some(((WorkKind::Child, child_id), phase)),
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::CommandTerminal { command_id, phase },
            }
            | NormalizedSessionLifecycle::Signal {
                signal: NormalizedLifecycleSignal::CommandPhase { command_id, phase },
            } => Some(((WorkKind::Command, command_id), phase)),
            _ => None,
        };
        if let Some((key, phase)) = entry {
            let mut turns = work
                .remove(&key)
                .map_or_else(BTreeSet::new, |work| work.turns);
            turns.extend(turn);
            work.insert(
                key,
                Work {
                    phase,
                    at: fact.cursor,
                    turns,
                },
            );
        }
    }
    Ok(work)
}

fn work_status(
    work: &OpenWork,
    kind: WorkKind,
    detached: bool,
    generating: bool,
) -> ConversationWorkStatus {
    let live = detached
        || work
            .iter()
            .any(|((entry, _), work)| *entry == kind && work.phase.is_live());
    match (live, generating) {
        (false, _) => ConversationWorkStatus::None,
        (true, true) => ConversationWorkStatus::Active,
        (true, false) => ConversationWorkStatus::Waiting,
    }
}

fn decode(payload: &str) -> Result<RunLifecycleFact, LedgerError> {
    serde_json::from_str(payload).map_err(|error| LedgerError::Storage(error.to_string()))
}

#[cfg(test)]
#[path = "run_live_status_plan_tests.rs"]
mod plan_tests;
