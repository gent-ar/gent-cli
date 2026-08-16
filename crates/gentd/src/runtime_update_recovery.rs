//! Explicit successor recovery composition after an external staged-runtime handoff.
//!
//! This module never downloads, stages, starts, or replaces a process. Its caller is already the
//! staged `gentd` process, holding the host lock after a separately supervised replacement.

use std::path::Path;

use gent_ports::{Ledger, runtime_update::RuntimeUpdateJournal};
use gent_runtime::{
    Coordinator, RuntimeUpdateAuthority, RuntimeUpdateSuccessor, RuntimeUpdateSuccessorRequest,
};
use gent_store::SqliteLedger;
use gent_types::{HostEpoch, RuntimeReleaseIdentity};

use crate::runtime_update_config;

/// Inputs accepted only from the explicit successor-recovery daemon profile.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeUpdateRecoverConfig<'a> {
    pub(crate) enabled: bool,
    pub(crate) attempt_id: Option<&'a str>,
    pub(crate) cache_path: Option<&'a Path>,
    pub(crate) trust_path: Option<&'a Path>,
    pub(crate) keys: &'a [String],
    pub(crate) now_unix_seconds: u64,
}

/// Confirms the already staged successor and atomically opens a new writer epoch.
///
/// # Errors
/// Returns an error unless the durable handoff, signed release cache, staged daemon version, and
/// closed old ingress all agree. An error leaves ingress closed.
pub(crate) fn recover_if_enabled(
    data_dir: &Path,
    config: RuntimeUpdateRecoverConfig<'_>,
) -> Result<Option<HostEpoch>, String> {
    if !config.enabled {
        return Ok(None);
    }
    let attempt_id = config
        .attempt_id
        .filter(|value| !value.trim().is_empty())
        .ok_or("runtime update recovery requires --runtime-update-attempt-id")?;
    let trusted = runtime_update_config::load_trusted(
        config.cache_path,
        config.trust_path,
        config.keys,
        config.now_unix_seconds,
    )?;
    if trusted.source.release().payload.release_version != trusted.context.current_version {
        return Err("staged gentd version does not match the signed recovery release".into());
    }
    let ledger = SqliteLedger::open(data_dir.join("gent.db")).map_err(|error| error.to_string())?;
    let ingress = ledger.host_ingress().map_err(|error| error.to_string())?;
    let record = ledger
        .find_runtime_update(attempt_id)
        .map_err(|error| error.to_string())?
        .ok_or("runtime update recovery attempt was not found")?;
    let receipt = record
        .handoff
        .staging_receipt
        .clone()
        .ok_or("runtime update recovery requires a durable staging receipt")?;
    let release = trusted.source.release();
    let identity = RuntimeReleaseIdentity {
        key_id: release.key_id.clone(),
        release_version: release.payload.release_version,
        target: release.payload.artifact.target.clone(),
        artifact_digest_sha256: release.payload.artifact.digest_sha256.clone(),
    };
    RuntimeUpdateSuccessor::new(ledger.clone(), RuntimeUpdateAuthority::Approved)
        .confirm(&RuntimeUpdateSuccessorRequest {
            attempt_id: attempt_id.into(),
            active_host_epoch: ingress.epoch,
            release: identity,
            staging_receipt: receipt,
        })
        .map_err(|error| error.to_string())?;
    let next = Coordinator::new(ledger, gent_types::CapabilitySet::default())
        .fence_and_open(ingress.epoch)
        .map_err(|error| error.to_string())?;
    Ok(Some(next.epoch))
}

#[cfg(test)]
#[path = "runtime_update_recovery_tests.rs"]
mod tests;
