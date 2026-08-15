use gent_ports::{DecisionClaim, DecisionPhaseUpdate, LedgerError};
use gent_types::{DecisionCommand, DecisionSettlement, DecisionSettlementPhase};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::queries::storage_error;

/// Creates a decision once across independent coordinators.
pub(super) fn claim(
    ledger: &SqliteLedger,
    command: &DecisionCommand,
) -> Result<DecisionClaim, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let by_key = transaction
        .query_row(
            "SELECT decision_id, idempotency_key, phase FROM decisions WHERE idempotency_key = ?1",
            [&command.idempotency_key],
            decode_decision,
        )
        .optional()
        .map_err(storage_error)?;
    let existing = match by_key {
        Some(decision) => Some(decision),
        None => transaction
            .query_row(
                "SELECT decision_id, idempotency_key, phase FROM decisions WHERE decision_id = ?1",
                [&command.decision_id],
                decode_decision,
            )
            .optional()
            .map_err(storage_error)?,
    };
    let claim = if let Some(decision) = existing {
        DecisionClaim::Existing(decision)
    } else {
        let decision = DecisionSettlement {
            decision_id: command.decision_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            phase: DecisionSettlementPhase::Pending,
        };
        transaction
            .execute(
                "INSERT INTO decisions (decision_id, idempotency_key, phase) VALUES (?1, ?2, ?3)",
                params![
                    decision.decision_id,
                    decision.idempotency_key,
                    encode_phase(&decision.phase)
                ],
            )
            .map_err(storage_error)?;
        DecisionClaim::Created(decision)
    };
    transaction.commit().map_err(storage_error)?;
    Ok(claim)
}

/// Reads one decision without granting mutation authority.
pub(super) fn find(
    ledger: &SqliteLedger,
    decision_id: &str,
) -> Result<Option<DecisionSettlement>, LedgerError> {
    let connection = ledger.lock()?;
    connection
        .query_row(
            "SELECT decision_id, idempotency_key, phase FROM decisions WHERE decision_id = ?1",
            [decision_id],
            decode_decision,
        )
        .optional()
        .map_err(storage_error)
}

/// Applies an optimistic transition and returns the winner when concurrently advanced.
pub(super) fn replace_phase(
    ledger: &SqliteLedger,
    decision_id: &str,
    expected: &DecisionSettlementPhase,
    next: &DecisionSettlementPhase,
) -> Result<DecisionPhaseUpdate, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let changed = transaction
        .execute(
            "UPDATE decisions SET phase = ?1 WHERE decision_id = ?2 AND phase = ?3",
            params![encode_phase(next), decision_id, encode_phase(expected)],
        )
        .map_err(storage_error)?;
    let decision = transaction
        .query_row(
            "SELECT decision_id, idempotency_key, phase FROM decisions WHERE decision_id = ?1",
            [decision_id],
            decode_decision,
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| LedgerError::Invariant("cannot update an unknown decision".into()))?;
    transaction.commit().map_err(storage_error)?;
    Ok(if changed == 1 {
        DecisionPhaseUpdate::Applied(decision)
    } else {
        DecisionPhaseUpdate::Current(decision)
    })
}

fn decode_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<DecisionSettlement> {
    Ok(DecisionSettlement {
        decision_id: row.get(0)?,
        idempotency_key: row.get(1)?,
        phase: decode_phase(&row.get::<_, String>(2)?)?,
    })
}

fn encode_phase(phase: &DecisionSettlementPhase) -> &'static str {
    match phase {
        DecisionSettlementPhase::Pending => "pending",
        DecisionSettlementPhase::Acknowledged => "acknowledged",
        DecisionSettlementPhase::Settled => "settled",
        DecisionSettlementPhase::Unprovable => "unprovable",
        DecisionSettlementPhase::RecoveryRequired => "recoveryRequired",
    }
}

fn decode_phase(value: &str) -> rusqlite::Result<DecisionSettlementPhase> {
    match value {
        "pending" => Ok(DecisionSettlementPhase::Pending),
        "acknowledged" => Ok(DecisionSettlementPhase::Acknowledged),
        "settled" => Ok(DecisionSettlementPhase::Settled),
        "unprovable" => Ok(DecisionSettlementPhase::Unprovable),
        "recoveryRequired" => Ok(DecisionSettlementPhase::RecoveryRequired),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
