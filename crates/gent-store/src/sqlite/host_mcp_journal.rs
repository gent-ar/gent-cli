//! Fenced, durable at-most-once admission for host MCP calls.
use super::{
    SqliteLedger,
    epoch::require_epoch,
    queries::{host_ingress, storage_error},
};
use gent_ports::{HostMcpAdmission, HostMcpJournal, HostMcpScope, IngressMode, LedgerError};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

const MAX_BYTES: usize = 1024 * 1024;
fn invalid() -> LedgerError {
    LedgerError::Invariant("host MCP scope or operation is invalid".into())
}
fn validate(scope: &HostMcpScope, key: &str) -> Result<(), LedgerError> {
    let text = [
        &scope.conversation_id,
        &scope.run_id,
        &scope.turn_id,
        &scope.coordinator_id,
        &scope.policy_id,
    ];
    if text
        .iter()
        .any(|v| v.is_empty() || v.len() > 512 || v.contains('\0'))
        || key.is_empty()
        || key.len() > 512
        || key.contains('\0')
        || scope.source_sha256.len() != 64
        || !scope.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || scope.generation.len() != 32
        || !scope.generation.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    Ok(())
}
fn fence(tx: &Transaction<'_>, scope: &HostMcpScope) -> Result<(), LedgerError> {
    let ingress = host_ingress(tx)?;
    require_epoch(scope.host_epoch, ingress.epoch)?;
    if ingress.mode != IngressMode::Open {
        return Err(invalid());
    }
    let owned: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns t JOIN runs r ON r.run_id=t.run_id JOIN run_leases l ON l.run_id=r.run_id JOIN agent_chat_conversations c ON c.conversation_id=t.conversation_id JOIN policies p ON p.workspace_id=c.workspace_id WHERE t.turn_id=?1 AND t.conversation_id=?2 AND t.run_id=?3 AND r.conversation_id=?2 AND t.phase IN ('active','waitingPermission') AND l.coordinator_id=?4 AND l.host_epoch=?5 AND p.policy_id=?6 AND p.scope='providerPermissions' AND p.revision=(SELECT MAX(p2.revision) FROM policies p2 WHERE p2.workspace_id=p.workspace_id AND p2.scope=p.scope))",
        params![scope.turn_id, scope.conversation_id, scope.run_id, scope.coordinator_id, scope.host_epoch.0, scope.policy_id], |row| row.get(0)).map_err(storage_error)?;
    if !owned {
        return Err(invalid());
    }
    Ok(())
}
// Every transition matches the complete original authority, including generation.
fn binding(scope: &HostMcpScope) -> String {
    let mut hash = Sha256::new();
    for part in [
        &scope.conversation_id,
        &scope.run_id,
        &scope.turn_id,
        &scope.coordinator_id,
        &scope.host_epoch.0.to_string(),
        &scope.policy_id,
        &scope.source_sha256,
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
type RecordedOperation = (String, String, String, Option<Vec<u8>>);

fn recorded(
    tx: &Transaction<'_>,
    scope: &HostMcpScope,
    key: &str,
) -> Result<Option<RecordedOperation>, LedgerError> {
    tx.query_row("SELECT binding_sha256, request_sha256, state, response FROM host_mcp_operations WHERE generation=?1 AND request_key=?2",
        params![scope.generation,key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .optional().map_err(storage_error)
}
impl HostMcpJournal for SqliteLedger {
    fn reserve_host_mcp(
        &self,
        scope: &HostMcpScope,
        key: &str,
        request: &[u8],
    ) -> Result<HostMcpAdmission, LedgerError> {
        validate(scope, key)?;
        if request.is_empty() || request.len() > MAX_BYTES {
            return Err(invalid());
        }
        let digest = format!("{:x}", Sha256::digest(request));
        let mut conn = self.lock()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        fence(&tx, scope)?;
        if let Some((owner, hash, state, response)) = recorded(&tx, scope, key)? {
            if owner != binding(scope) || hash != digest {
                return Err(invalid());
            }
            if state == "completed" {
                return Ok(HostMcpAdmission::Completed(response.ok_or_else(invalid)?));
            }
            tx.execute("UPDATE host_mcp_operations SET state='unknown' WHERE generation=?1 AND request_key=?2 AND state='pending'",params![scope.generation,key]).map_err(storage_error)?;
            tx.commit().map_err(storage_error)?;
            return Ok(HostMcpAdmission::Unknown);
        }
        let count: u64 = tx
            .query_row(
                "SELECT COUNT(*) FROM host_mcp_operations WHERE turn_id=?1",
                [&scope.turn_id],
                |r| r.get(0),
            )
            .map_err(storage_error)?;
        if count >= 256 {
            return Err(invalid());
        }
        tx.execute("INSERT INTO host_mcp_operations(generation,request_key,conversation_id,run_id,turn_id,coordinator_id,host_epoch,policy_id,source_sha256,request_sha256,binding_sha256,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'pending')",
            params![scope.generation,key,scope.conversation_id,scope.run_id,scope.turn_id,scope.coordinator_id,scope.host_epoch.0,scope.policy_id,scope.source_sha256,digest,binding(scope)]).map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
        Ok(HostMcpAdmission::Fresh)
    }
    fn complete_host_mcp(
        &self,
        scope: &HostMcpScope,
        key: &str,
        response: &[u8],
    ) -> Result<(), LedgerError> {
        validate(scope, key)?;
        if response.is_empty() || response.len() > MAX_BYTES {
            return Err(invalid());
        }
        let mut conn = self.lock()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        fence(&tx, scope)?;
        let (owner, _, state, _) = recorded(&tx, scope, key)?.ok_or_else(invalid)?;
        if owner != binding(scope) || state != "pending" {
            return Err(invalid());
        }
        let retained:u64=tx.query_row("SELECT COALESCE(SUM(length(response)),0) FROM host_mcp_operations WHERE turn_id=?1",[&scope.turn_id],|r|r.get(0)).map_err(storage_error)?;
        if retained.saturating_add(response.len() as u64) > 32 * 1024 * 1024 {
            return Err(invalid());
        }
        tx.execute("UPDATE host_mcp_operations SET state='completed',response=?3 WHERE generation=?1 AND request_key=?2 AND state='pending'",params![scope.generation,key,response]).map_err(storage_error)?;
        tx.commit().map_err(storage_error)
    }
    fn unknown_host_mcp(&self, scope: &HostMcpScope, key: &str) -> Result<(), LedgerError> {
        validate(scope, key)?;
        let mut conn = self.lock()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let (owner, _, _, _) = recorded(&tx, scope, key)?.ok_or_else(invalid)?;
        if owner != binding(scope) {
            return Err(invalid());
        }
        tx.execute("UPDATE host_mcp_operations SET state='unknown',response=NULL WHERE generation=?1 AND request_key=?2 AND state='pending'",params![scope.generation,key]).map_err(storage_error)?;
        tx.commit().map_err(storage_error)
    }
}

#[cfg(test)]
#[path = "host_mcp_journal_tests.rs"]
mod tests;
