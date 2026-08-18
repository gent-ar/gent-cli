use gent_ports::{
    ClaurstCheckpoint, ClaurstDrainBatch, ClaurstDrainRequest, ClaurstSessionBinding,
    ClaurstSourceId, ClaurstTerminal, Ledger, RunCheckpointLedger,
};
use gent_runtime::{Coordinator, RuntimeError};
use sha2::{Digest, Sha256};

pub(super) fn validate_binding(binding: &ClaurstSessionBinding) -> Result<(), RuntimeError> {
    if binding.run_id.trim().is_empty()
        || binding.source_id.0.trim().is_empty()
        || binding.opaque_session_id.is_empty()
    {
        return Err(invariant("private Claurst binding identity is required"));
    }
    Ok(())
}

pub(super) fn validate_batch(
    request: &ClaurstDrainRequest,
    binding: &ClaurstSessionBinding,
    batch: &ClaurstDrainBatch,
) -> Result<ClaurstCheckpoint, RuntimeError> {
    if batch.facts.len() > usize::from(request.limit) || !request.is_bounded() {
        return Err(invariant("private Claurst batch exceeds its bound"));
    }
    if batch
        .session_binding
        .as_ref()
        .is_some_and(|session| session != binding)
    {
        return Err(invariant(
            "private Claurst batch attempted to change its session",
        ));
    }
    let mut cursor = request.after_cursor;
    for fact in &batch.facts {
        if fact.source_id != request.source_id || fact.cursor <= cursor {
            return Err(invariant("private Claurst facts are not source-ordered"));
        }
        cursor = fact.cursor;
    }
    let checkpoint = batch
        .checkpoint
        .clone()
        .ok_or_else(|| invariant("private Claurst drain requires a checkpoint"))?;
    if checkpoint.run_id != request.run_id
        || checkpoint.source_id != request.source_id
        || checkpoint.cursor != cursor
        || !digest(&checkpoint.state_digest_sha256)
    {
        return Err(invariant(
            "private Claurst checkpoint does not seal this drain",
        ));
    }
    Ok(checkpoint)
}

pub(super) fn restored<L: Ledger + RunCheckpointLedger>(
    coordinator: &Coordinator<L>,
    binding: &ClaurstSessionBinding,
) -> Result<(u64, bool), RuntimeError> {
    let prefix = format!("claurst:{}:", source_hash(&binding.source_id));
    let record = coordinator
        .run_checkpoints(&binding.run_id)?
        .into_iter()
        .rev()
        .find(|item| item.checkpoint_id.starts_with(&prefix));
    record.map_or(Ok((0, false)), |item| {
        parse_checkpoint(&item.checkpoint_id, &prefix)
    })
}

pub(super) fn checkpoint_id(source_id: &ClaurstSourceId, cursor: u64, terminal: bool) -> String {
    format!(
        "claurst:{}:{cursor}:{}",
        source_hash(source_id),
        if terminal { "terminal" } else { "open" }
    )
}

pub(super) fn event_id(source_id: &ClaurstSourceId, kind: &str) -> String {
    format!("claurst:{}:{kind}", source_hash(source_id))
}

pub(super) fn terminal_name(terminal: ClaurstTerminal) -> &'static str {
    match terminal {
        ClaurstTerminal::Completed => "completed",
        ClaurstTerminal::Interrupted => "interrupted",
        ClaurstTerminal::Failed { .. } => "failed",
    }
}

pub(super) fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

fn parse_checkpoint(id: &str, prefix: &str) -> Result<(u64, bool), RuntimeError> {
    let Some((cursor, state)) = id
        .strip_prefix(prefix)
        .and_then(|tail| tail.split_once(':'))
    else {
        return Err(invariant("private Claurst checkpoint identity is invalid"));
    };
    let cursor = cursor
        .parse()
        .map_err(|_| invariant("private Claurst checkpoint cursor is invalid"))?;
    match state {
        "open" => Ok((cursor, false)),
        "terminal" => Ok((cursor, true)),
        _ => Err(invariant("private Claurst checkpoint state is invalid")),
    }
}

fn source_hash(source_id: &ClaurstSourceId) -> String {
    hex::encode(Sha256::digest(source_id.0.as_bytes()))
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
