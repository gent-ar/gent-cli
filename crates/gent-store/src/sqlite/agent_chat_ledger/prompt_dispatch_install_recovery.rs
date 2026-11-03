//! Repair legacy installer-domain ambiguity without replaying an installation.

use gent_ports::LedgerError;
use gent_types::{Command, Event, HostEpoch, ProviderPromptProvisionCommandBinding, ReceiptId};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::sqlite::queries::{command_fingerprint, storage_error};

pub(super) fn restore(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    recover_interrupted_installers(transaction, host_epoch)?;
    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT r.receipt_id,r.idempotency_key,r.host_epoch,r.payload_digest,e.payload
             FROM receipts r JOIN events e ON e.receipt_id=r.receipt_id AND e.host_epoch=r.host_epoch
             WHERE r.kind='providerPromptProvision' AND r.status='unprovable'
              AND e.kind='privatePromptProvisionUnprovable'
              AND (SELECT count(*) FROM events terminal WHERE terminal.receipt_id=r.receipt_id
               AND terminal.kind LIKE 'privatePromptProvision%' AND terminal.kind!='privatePromptProvisionAccepted')=1",
        ).map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
    };
    for (receipt, key, epoch, fingerprint, encoded) in candidates {
        let Ok(binding) = serde_json::from_str::<ProviderPromptProvisionCommandBinding>(&encoded)
        else {
            continue;
        };
        if !binding.is_valid()
            || !binding.prompt.consent_granted
            || binding.prompt.reviewed_plan_digest != binding.expected_reviewed_plan_digest
        {
            continue;
        }
        let payload = serde_json::to_value(&binding).map_err(storage_error)?;
        if serde_json::from_str::<serde_json::Value>(&encoded)
            .ok()
            .as_ref()
            != Some(&payload)
        {
            continue;
        }
        let command = Command {
            receipt_id: ReceiptId(receipt),
            idempotency_key: key,
            host_epoch: HostEpoch(epoch),
            kind: "providerPromptProvision".into(),
            payload,
        };
        if fingerprint.is_empty() || fingerprint != command_fingerprint(&command)? {
            continue;
        }
        restore_exact(transaction, &binding, &command)?;
    }
    Ok(())
}

fn restore_exact(
    transaction: &Transaction<'_>,
    binding: &ProviderPromptProvisionCommandBinding,
    command: &Command,
) -> Result<(), LedgerError> {
    // A canonical acceptance and terminal event must bind the same durable command.
    let accepted: u64 = transaction
        .query_row(
            "SELECT count(*) FROM events WHERE receipt_id=?1 AND host_epoch=?2
         AND kind='privatePromptProvisionAccepted'
         AND CASE WHEN json_valid(payload) THEN json(payload)=json(?3) ELSE 0 END
         AND (SELECT count(*) FROM events admission WHERE admission.receipt_id=?1
          AND admission.kind='privatePromptProvisionAccepted')=1",
            params![
                command.receipt_id.0,
                command.host_epoch.0,
                command.payload.to_string()
            ],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if accepted != 1 {
        return Ok(());
    }
    if let Some(message) = held_message(transaction, binding, "unprovable", None)? {
        transaction.execute("UPDATE agent_chat_prompt_dispatches SET state='awaiting_readiness' WHERE message_id=?1", [message]).map_err(storage_error)?;
    }
    Ok(())
}

fn held_message(
    transaction: &Transaction<'_>,
    binding: &ProviderPromptProvisionCommandBinding,
    state: &str,
    ignored_receipt: Option<&str>,
) -> Result<Option<String>, LedgerError> {
    transaction.query_row(
        "SELECT m.message_id FROM agent_chat_prompt_dispatches d
           JOIN conversation_messages m ON m.message_id=d.message_id
           JOIN agent_chat_prompt_receipts p ON p.message_id=m.message_id
            AND p.run_id=m.run_id AND p.turn_id=m.turn_id AND p.conversation_id=m.conversation_id
           JOIN receipts r ON r.idempotency_key=p.idempotency_key AND r.status='settled'
           JOIN turns t ON t.turn_id=p.turn_id AND t.run_id=p.run_id AND t.conversation_id=p.conversation_id
           JOIN agent_chat_run_selections s ON s.run_id=p.run_id
           WHERE r.receipt_id=?1 AND p.conversation_id=?2 AND p.run_id=?3 AND s.provider=?4
            AND d.state=?5 AND d.coordinator_id IS NULL AND d.host_epoch IS NULL
            AND t.phase='active' AND p.disposition IN ('send','queue')
            AND NOT EXISTS(SELECT 1 FROM agent_chat_steer_targets target WHERE target.message_id=m.message_id)
            AND p.run_id=(SELECT current.run_id FROM runs current
             JOIN agent_chat_run_selections selected ON selected.run_id=current.run_id
             WHERE current.conversation_id=p.conversation_id ORDER BY current.rowid DESC LIMIT 1)
            AND NOT EXISTS(SELECT 1 FROM receipts installing JOIN events admission
             ON admission.receipt_id=installing.receipt_id AND admission.host_epoch=installing.host_epoch
             WHERE installing.kind='providerPromptProvision' AND installing.status='accepted'
              AND admission.kind='privatePromptProvisionAccepted' AND json_valid(admission.payload)
              AND json_extract(admission.payload,'$.prompt.promptReceiptId')=?1
              AND (?6 IS NULL OR installing.receipt_id<>?6))",
        params![binding.prompt.prompt_receipt_id.0, binding.prompt.conversation_id.0,
            binding.prompt.run_id.0, binding.prompt.provider, state, ignored_receipt], |row| row.get(0),
    ).optional().map_err(storage_error)
}

/// Records the successor's uncertainty, never an invented terminal fact from the old owner.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InterruptedInstall<'a> {
    original_command: &'a Command,
    original_command_fingerprint: &'a str,
    recovery_host_epoch: HostEpoch,
}

fn recover_interrupted_installers(
    transaction: &Transaction<'_>,
    epoch: HostEpoch,
) -> Result<(), LedgerError> {
    // require_open has already established the successor epoch in this transaction.
    // An orphan npm child may still exist: its old prefix is left untouched and its
    // result remains unknown. A future explicit retry must use an isolated prefix.
    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT r.receipt_id,r.idempotency_key,r.host_epoch,r.payload_digest,e.payload
             FROM receipts r JOIN events e ON e.receipt_id=r.receipt_id AND e.host_epoch=r.host_epoch
             WHERE r.kind='providerPromptProvision' AND r.status='accepted' AND r.host_epoch<?1
              AND e.kind='privatePromptProvisionAccepted'
              AND (SELECT count(*) FROM events a WHERE a.receipt_id=r.receipt_id AND a.kind='privatePromptProvisionAccepted')=1
              AND NOT EXISTS(SELECT 1 FROM events terminal WHERE terminal.receipt_id=r.receipt_id
               AND terminal.kind LIKE 'privatePromptProvision%' AND terminal.kind!='privatePromptProvisionAccepted')",
        ).map_err(storage_error)?;
        let rows = statement
            .query_map([epoch.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
    };
    for (receipt, key, original_epoch, fingerprint, encoded) in candidates {
        let Ok(binding) = serde_json::from_str::<ProviderPromptProvisionCommandBinding>(&encoded)
        else {
            continue;
        };
        if !binding.is_valid()
            || !binding.prompt.consent_granted
            || binding.prompt.reviewed_plan_digest != binding.expected_reviewed_plan_digest
        {
            continue;
        }
        let payload = serde_json::to_value(&binding).map_err(storage_error)?;
        if serde_json::from_str::<serde_json::Value>(&encoded)
            .ok()
            .as_ref()
            != Some(&payload)
        {
            continue;
        }
        let command = Command {
            receipt_id: ReceiptId(receipt),
            idempotency_key: key,
            host_epoch: HostEpoch(original_epoch),
            kind: "providerPromptProvision".into(),
            payload,
        };
        if fingerprint.is_empty() || fingerprint != command_fingerprint(&command)? {
            continue;
        }
        let Some(message) = held_message(
            transaction,
            &binding,
            "provisioning",
            Some(&command.receipt_id.0),
        )?
        else {
            continue;
        };
        transaction.execute("UPDATE receipts SET status='unprovable' WHERE idempotency_key=?1 AND status='accepted'", [&command.idempotency_key]).map_err(storage_error)?;
        crate::sqlite::queries::append_event(
            transaction,
            &Event {
                cursor: 0,
                event_id: format!(
                    "{}:prompt-provision:recovered:{}",
                    command.receipt_id.0, epoch.0
                ),
                receipt_id: command.receipt_id.clone(),
                host_epoch: epoch,
                kind: "privatePromptProvisionRecovered".into(),
                payload: serde_json::to_value(InterruptedInstall {
                    original_command: &command,
                    original_command_fingerprint: &fingerprint,
                    recovery_host_epoch: epoch,
                })
                .map_err(storage_error)?,
            },
        )?;
        transaction.execute("UPDATE agent_chat_prompt_dispatches SET state='awaiting_readiness' WHERE message_id=?1", [message]).map_err(storage_error)?;
    }
    Ok(())
}
