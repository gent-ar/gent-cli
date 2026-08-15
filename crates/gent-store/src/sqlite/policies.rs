//! `SQLite` persistence for immutable workspace permission-policy revisions.

use gent_ports::LedgerError;
use gent_types::{PolicyRecord, PolicyScope};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn save(ledger: &SqliteLedger, policy: &PolicyRecord) -> Result<(), LedgerError> {
    validate(policy)?;
    let connection = ledger.lock()?;
    let latest = current(&connection, &policy.workspace_id, policy.scope)?;
    let next = latest.as_ref().map_or(Ok(1), |current| {
        current
            .revision
            .checked_add(1)
            .ok_or_else(|| LedgerError::Invariant("policy revision overflow".into()))
    })?;
    if policy.revision != next {
        return Err(LedgerError::Invariant(format!(
            "policy revision must be {next} for this workspace and scope"
        )));
    }
    let allowed_tools = serde_json::to_string(&policy.allowed_tools)
        .map_err(|error| LedgerError::Storage(error.to_string()))?;
    connection
        .execute(
            "INSERT INTO policies (policy_id, workspace_id, scope, revision, allowed_tools) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![policy.policy_id, policy.workspace_id, encode_scope(policy.scope), policy.revision, allowed_tools],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn current_policy(
    ledger: &SqliteLedger,
    workspace_id: &str,
    scope: PolicyScope,
) -> Result<Option<PolicyRecord>, LedgerError> {
    let connection = ledger.lock()?;
    current(&connection, workspace_id, scope)
}

fn current(
    connection: &rusqlite::Connection,
    workspace_id: &str,
    scope: PolicyScope,
) -> Result<Option<PolicyRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT policy_id, workspace_id, scope, revision, allowed_tools FROM policies WHERE workspace_id = ?1 AND scope = ?2 ORDER BY revision DESC LIMIT 1",
            params![workspace_id, encode_scope(scope)],
            decode_policy,
        )
        .optional()
        .map_err(storage_error)
}

fn validate(policy: &PolicyRecord) -> Result<(), LedgerError> {
    if policy.policy_id.is_empty() || policy.workspace_id.is_empty() || policy.revision == 0 {
        return Err(LedgerError::Invariant(
            "policy identity, workspace, and non-zero revision are required".into(),
        ));
    }
    if policy
        .allowed_tools
        .iter()
        .any(|tool| !valid_tool_name(tool))
        || policy
            .allowed_tools
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(LedgerError::Invariant(
            "policy tools must be non-empty, canonical, sorted, and unique".into(),
        ));
    }
    Ok(())
}

fn valid_tool_name(tool: &str) -> bool {
    !tool.is_empty()
        && tool.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn encode_scope(scope: PolicyScope) -> &'static str {
    match scope {
        PolicyScope::ProviderPermissions => "providerPermissions",
    }
}

fn decode_policy(row: &rusqlite::Row<'_>) -> rusqlite::Result<PolicyRecord> {
    let tools = row.get::<_, String>(4)?;
    Ok(PolicyRecord {
        policy_id: row.get(0)?,
        workspace_id: row.get(1)?,
        scope: decode_scope(&row.get::<_, String>(2)?)?,
        revision: row.get(3)?,
        allowed_tools: serde_json::from_str(&tools).map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

fn decode_scope(scope: &str) -> rusqlite::Result<PolicyScope> {
    match scope {
        "providerPermissions" => Ok(PolicyScope::ProviderPermissions),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
