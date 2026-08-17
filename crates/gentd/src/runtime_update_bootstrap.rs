//! One-shot verification for runtime-update material before an installer selects a pair.

use std::{fs, path::Path};

use gent_runtime::{CachedRuntimeRelease, RuntimeReleaseTrust};
use gent_types::{RuntimeVersion, SignedRuntimeRelease};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::runtime_update_config;

/// Exact filesystem inputs accepted only by the staged runtime bootstrap mode.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeUpdateBootstrapConfig<'a> {
    pub(crate) enabled: bool,
    pub(crate) cache_path: Option<&'a Path>,
    pub(crate) trust_path: Option<&'a Path>,
    pub(crate) release_path: Option<&'a Path>,
    pub(crate) archive_path: Option<&'a Path>,
    pub(crate) archive_manifest_path: Option<&'a Path>,
    pub(crate) now_unix_seconds: u64,
}

/// Verifies and atomically creates the local cache for the staged exact runtime.
///
/// # Errors
/// Returns an error without changing the selected runtime pair when material is missing,
/// symlinked, unsigned, expired, or inconsistent with the archive and staged binary version.
pub(crate) fn verify_if_enabled(config: RuntimeUpdateBootstrapConfig<'_>) -> Result<bool, String> {
    if !config.enabled {
        return Ok(false);
    }
    let cache = required(config.cache_path, "--runtime-release-cache")?;
    let trust_path = required(config.trust_path, "--runtime-release-trust")?;
    let release_path = required(config.release_path, "--runtime-release-manifest")?;
    let archive_path = required(config.archive_path, "--runtime-release-archive")?;
    let archive_manifest = required(
        config.archive_manifest_path,
        "--runtime-release-archive-manifest",
    )?;
    reject_link(cache, false)?;
    let trust = RuntimeReleaseTrust::new(runtime_update_config::load_keys_from_file(trust_path)?);
    let release: SignedRuntimeRelease = parse_file(release_path)?;
    let archive: ArchiveManifest = parse_file(archive_manifest)?;
    let bytes = real_file(archive_path)?;
    let current = runtime_update_config::package_version();
    let target = runtime_update_config::platform_target()?;
    validate_archive(&archive, archive_path, &bytes, current, &target)?;
    trust
        .verify_release(&release, config.now_unix_seconds)
        .map_err(|error| format!("runtime release manifest is not trusted: {error}"))?;
    if release.payload.release_version != current
        || release.payload.artifact.target != target
        || release.payload.artifact.archive_name != archive.archive.name
        || release.payload.artifact.digest_sha256 != archive.archive.sha256
        || release.payload.artifact.size_bytes != archive.archive.size
    {
        return Err("runtime release manifest does not bind the staged archive".into());
    }
    CachedRuntimeRelease::verify(release, &trust, config.now_unix_seconds)
        .and_then(|cached| cached.store(cache, &trust, config.now_unix_seconds))
        .map_err(|error| format!("runtime release cache could not be stored: {error}"))?;
    Ok(true)
}

fn required<'a>(value: Option<&'a Path>, flag: &str) -> Result<&'a Path, String> {
    value.ok_or_else(|| format!("runtime update bootstrap requires {flag}"))
}

fn real_file(path: &Path) -> Result<Vec<u8>, String> {
    reject_link(path, true)?;
    fs::read(path).map_err(|_| "runtime update bootstrap input is unreadable".into())
}

fn reject_link(path: &Path, required: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => Ok(()),
        Ok(_) => Err("runtime update bootstrap paths must be real files".into()),
        Err(_) if required => Err("runtime update bootstrap input is unavailable".into()),
        Err(_) => Ok(()),
    }
}

fn parse_file<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&real_file(path)?)
        .map_err(|_| "runtime update bootstrap metadata is invalid JSON".into())
}

fn validate_archive(
    manifest: &ArchiveManifest,
    archive_path: &Path,
    bytes: &[u8],
    current: RuntimeVersion,
    target: &str,
) -> Result<(), String> {
    let version = format!("v{}.{}.{}", current.major, current.minor, current.patch);
    let digest = hex::encode(Sha256::digest(bytes));
    let size = u64::try_from(bytes.len()).map_err(|_| "staged archive is too large")?;
    if manifest.schema_version != 1
        || manifest.version != version
        || manifest.target != target
        || archive_path.file_name().and_then(|name| name.to_str()) != Some(&manifest.archive.name)
        || manifest.archive.sha256 != digest
        || manifest.archive.size != size
        || manifest.binaries != vec!["gent".to_owned(), "gentd".to_owned()]
    {
        return Err("archive manifest does not bind the staged archive".into());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveManifest {
    schema_version: u16,
    version: String,
    target: String,
    archive: ArchiveIdentity,
    binaries: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchiveIdentity {
    name: String,
    sha256: String,
    size: u64,
}

#[cfg(test)]
#[path = "runtime_update_bootstrap_tests.rs"]
mod tests;
