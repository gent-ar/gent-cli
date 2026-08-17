//! `SQLite` persistence for immutable workspace permission-policy revisions.

use gent_ports::LedgerError;
use gent_types::{PermissionMode, PolicyRecord, PolicyScope};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn save(ledger: &SqliteLedger, policy: &PolicyRecord) -> Result<(), LedgerError> {
    validate(policy)?;
    let connection = ledger.lock()?;
    if let Some(existing) = by_id(&connection, &policy.policy_id)? {
        if existing == *policy {
            return Ok(());
        }
        return Err(LedgerError::Invariant(
            "policy identity is bound to another immutable revision".into(),
        ));
    }
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
    let allowed_categories = serde_json::to_string(&policy.allowed_categories)
        .map_err(|error| LedgerError::Storage(error.to_string()))?;
    connection
        .execute(
            "INSERT INTO policies (policy_id, workspace_id, scope, revision, allowed_tools, mode, allowed_categories) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![policy.policy_id, policy.workspace_id, encode_scope(policy.scope), policy.revision, allowed_tools, encode_mode(policy.mode), allowed_categories],
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
            "SELECT policy_id, workspace_id, scope, revision, allowed_tools, mode, allowed_categories FROM policies WHERE workspace_id = ?1 AND scope = ?2 ORDER BY revision DESC LIMIT 1",
            params![workspace_id, encode_scope(scope)],
            decode_policy,
        )
        .optional()
        .map_err(storage_error)
}

fn by_id(
    connection: &rusqlite::Connection,
    policy_id: &str,
) -> Result<Option<PolicyRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT policy_id, workspace_id, scope, revision, allowed_tools, mode, allowed_categories FROM policies WHERE policy_id = ?1",
            [policy_id],
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
            "policy tools must be canonical, sorted, and unique".into(),
        ));
    }
    if policy
        .allowed_categories
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(LedgerError::Invariant(
            "policy categories must be canonical, sorted, and unique".into(),
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

const fn encode_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::Plan => "plan",
        PermissionMode::AutoAcceptEdits => "autoAcceptEdits",
        PermissionMode::Autonomous => "autonomous",
        PermissionMode::Bypass => "bypass",
    }
}

fn decode_policy(row: &rusqlite::Row<'_>) -> rusqlite::Result<PolicyRecord> {
    let tools = row.get::<_, String>(4)?;
    let categories = row.get::<_, String>(6)?;
    Ok(PolicyRecord {
        policy_id: row.get(0)?,
        workspace_id: row.get(1)?,
        scope: decode_scope(&row.get::<_, String>(2)?)?,
        revision: row.get(3)?,
        allowed_tools: serde_json::from_str(&tools).map_err(|_| rusqlite::Error::InvalidQuery)?,
        mode: decode_mode(&row.get::<_, String>(5)?)?,
        allowed_categories: serde_json::from_str(&categories)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

fn decode_mode(mode: &str) -> rusqlite::Result<PermissionMode> {
    match mode {
        "default" => Ok(PermissionMode::Default),
        "plan" => Ok(PermissionMode::Plan),
        "autoAcceptEdits" => Ok(PermissionMode::AutoAcceptEdits),
        "autonomous" => Ok(PermissionMode::Autonomous),
        "bypass" => Ok(PermissionMode::Bypass),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn decode_scope(scope: &str) -> rusqlite::Result<PolicyScope> {
    match scope {
        "providerPermissions" => Ok(PolicyScope::ProviderPermissions),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
