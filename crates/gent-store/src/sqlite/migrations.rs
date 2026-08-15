use gent_ports::LedgerError;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

const BASE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS host_state (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), epoch INTEGER NOT NULL, ingress TEXT NOT NULL DEFAULT 'open');
CREATE TABLE IF NOT EXISTS receipts (idempotency_key TEXT PRIMARY KEY NOT NULL, receipt_id TEXT NOT NULL UNIQUE, status TEXT NOT NULL, host_epoch INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS decisions (decision_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL UNIQUE, phase TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS event_snapshots (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), cursor INTEGER NOT NULL, host_epoch INTEGER NOT NULL, schema_version INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS runs (run_id TEXT PRIMARY KEY NOT NULL, parent_run_id TEXT REFERENCES runs(run_id), provider TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS run_version_locks (run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id), provider TEXT NOT NULL, canonical_path TEXT NOT NULL, file_identity TEXT NOT NULL, digest_sha256 TEXT NOT NULL, version TEXT NOT NULL, compatibility_entry TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS run_leases (run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id), coordinator_id TEXT NOT NULL, host_epoch INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS worktree_leases (worktree_id TEXT PRIMARY KEY NOT NULL, run_id TEXT NOT NULL REFERENCES runs(run_id), lease_token TEXT NOT NULL UNIQUE, host_epoch INTEGER NOT NULL);
";

const RUN_SESSION_BINDINGS: &str = "
CREATE TABLE IF NOT EXISTS run_session_bindings (run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id), provider_session_id TEXT NOT NULL);
";

const RUN_PROJECTIONS: &str = "
CREATE TABLE IF NOT EXISTS run_projections (run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id), host_epoch INTEGER NOT NULL, cursor INTEGER NOT NULL, payload TEXT NOT NULL);
";

const CONVERSATIONS_AND_TURNS: &str = "
CREATE TABLE IF NOT EXISTS conversations (conversation_id TEXT PRIMARY KEY NOT NULL);
ALTER TABLE runs ADD COLUMN conversation_id TEXT REFERENCES conversations(conversation_id);
CREATE TABLE IF NOT EXISTS turns (turn_id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id), run_id TEXT NOT NULL REFERENCES runs(run_id), sequence INTEGER NOT NULL CHECK (sequence > 0), phase TEXT NOT NULL, UNIQUE (run_id, sequence));
CREATE INDEX IF NOT EXISTS turns_by_run_sequence ON turns (run_id, sequence);
";

const CONVERSATION_ARTIFACTS: &str = "
CREATE TABLE IF NOT EXISTS conversation_artifacts (
    artifact_id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id),
    kind TEXT NOT NULL,
    source_turn_ids TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_version TEXT NOT NULL,
    input_digest TEXT NOT NULL,
    status TEXT NOT NULL,
    text TEXT,
    supersedes_artifact_id TEXT REFERENCES conversation_artifacts(artifact_id)
);
CREATE INDEX IF NOT EXISTS conversation_artifacts_by_conversation ON conversation_artifacts (conversation_id);
";

const CAPABILITY_CATALOG: &str = "
CREATE TABLE IF NOT EXISTS capability_catalog (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), schema_version INTEGER NOT NULL, capabilities TEXT NOT NULL);
";

const WORKSPACE_HIERARCHY: &str = "
CREATE TABLE IF NOT EXISTS workspaces (workspace_id TEXT PRIMARY KEY NOT NULL, canonical_path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS repositories (repository_id TEXT PRIMARY KEY NOT NULL, workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id), canonical_path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS worktrees (worktree_id TEXT PRIMARY KEY NOT NULL, repository_id TEXT NOT NULL REFERENCES repositories(repository_id), canonical_path TEXT NOT NULL UNIQUE);
";

#[derive(Debug)]
struct Migration {
    version: i64,
    sql: &'static str,
}

const MIGRATIONS: [Migration; 7] = [
    Migration {
        version: 1,
        sql: BASE_SCHEMA,
    },
    Migration {
        version: 2,
        sql: RUN_SESSION_BINDINGS,
    },
    Migration {
        version: 3,
        sql: RUN_PROJECTIONS,
    },
    Migration {
        version: 4,
        sql: CONVERSATIONS_AND_TURNS,
    },
    Migration {
        version: 5,
        sql: CONVERSATION_ARTIFACTS,
    },
    Migration {
        version: 6,
        sql: CAPABILITY_CATALOG,
    },
    Migration {
        version: 7,
        sql: WORKSPACE_HIERARCHY,
    },
];

pub(super) fn apply(connection: &mut Connection) -> Result<(), LedgerError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY CHECK (version > 0), checksum TEXT NOT NULL);",
        )
        .map_err(storage_error)?;
    verify_recorded(&transaction)?;
    for migration in &MIGRATIONS {
        if !is_recorded(&transaction, migration.version)? {
            transaction
                .execute_batch(migration.sql)
                .map_err(storage_error)?;
            ensure_ingress(&transaction)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO host_state (singleton, epoch, ingress) VALUES (1, 1, 'open')",
                    [],
                )
                .map_err(storage_error)?;
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version, checksum) VALUES (?1, ?2)",
                    (migration.version, checksum(migration)),
                )
                .map_err(storage_error)?;
        }
    }
    transaction.commit().map_err(storage_error)
}

fn verify_recorded(transaction: &Transaction<'_>) -> Result<(), LedgerError> {
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        let recorded = transaction
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = ?1",
                [migration.version],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        if let Some(recorded) = &recorded
            && recorded != &checksum(migration)
        {
            return Err(LedgerError::Invariant(format!(
                "migration {} checksum differs from the recorded schema",
                migration.version
            )));
        }
        if recorded.is_none() && has_later_record(transaction, MIGRATIONS[index + 1..].as_ref())? {
            return Err(LedgerError::Invariant(
                "schema migration history has a gap".into(),
            ));
        }
    }
    let unknown = transaction
        .query_row(
            "SELECT version FROM schema_migrations WHERE version > ?1 LIMIT 1",
            [MIGRATIONS.last().map_or(0, |migration| migration.version)],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(storage_error)?;
    if let Some(version) = unknown {
        return Err(LedgerError::Invariant(format!(
            "database requires unknown schema migration {version}"
        )));
    }
    Ok(())
}

fn has_later_record(
    transaction: &Transaction<'_>,
    later: &[Migration],
) -> Result<bool, LedgerError> {
    for migration in later {
        if is_recorded(transaction, migration.version)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_recorded(transaction: &Transaction<'_>, version: i64) -> Result<bool, LedgerError> {
    transaction
        .query_row(
            "SELECT 1 FROM schema_migrations WHERE version = ?1",
            [version],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(storage_error)
}

fn ensure_ingress(transaction: &Transaction<'_>) -> Result<(), LedgerError> {
    if !has_column(transaction, "host_state", "ingress")? {
        transaction
            .execute(
                "ALTER TABLE host_state ADD COLUMN ingress TEXT NOT NULL DEFAULT 'open'",
                [],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, LedgerError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(storage_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(storage_error)?;
    Ok(columns.filter_map(Result::ok).any(|name| name == column))
}

fn checksum(migration: &Migration) -> String {
    let mut digest = Sha256::new();
    digest.update(migration.version.to_le_bytes());
    digest.update(migration.sql.as_bytes());
    format!("{:x}", digest.finalize())
}

fn storage_error(error: impl std::fmt::Display) -> LedgerError {
    LedgerError::Storage(error.to_string())
}
