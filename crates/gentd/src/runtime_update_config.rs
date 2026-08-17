//! Daemon composition for explicitly configured, cached runtime-update metadata.
//!
//! This module is intentionally limited to loading already-verified metadata.
//! It does not fetch, persist, stage, health-check, or activate a runtime.

use std::{collections::BTreeMap, fs, path::Path};

use ed25519_dalek::VerifyingKey;
use gent_ports::runtime_update::{RuntimeReleaseSource, RuntimeUpdatePortError};
use gent_runtime::{
    CachedRuntimeRelease, RuntimeReleaseTrust, RuntimeUpdateCheckAuthority,
    RuntimeUpdateCheckContext, RuntimeUpdateCheckService, parse_trust_document,
};
use gent_types::{PROTOCOL_MAX, RuntimeReleaseChannel, RuntimeVersion, SignedRuntimeRelease};

/// Concrete daemon checker assembled only after cache and trust configuration validate.
pub(crate) type DaemonRuntimeUpdateChecks = RuntimeUpdateCheckService<CachedReleaseSource>;

/// Verified local release material shared by the check and plan compositions.
#[derive(Clone, Debug)]
pub(crate) struct TrustedRuntimeRelease {
    pub(crate) source: CachedReleaseSource,
    pub(crate) trust: RuntimeReleaseTrust,
    pub(crate) context: RuntimeUpdateCheckContext,
}

/// Immutable source backed by a release cache validated at daemon startup.
#[derive(Clone, Debug)]
pub(crate) struct CachedReleaseSource {
    release: SignedRuntimeRelease,
}

impl CachedReleaseSource {
    #[must_use]
    pub(crate) fn release(&self) -> &SignedRuntimeRelease {
        &self.release
    }
}

impl RuntimeReleaseSource for CachedReleaseSource {
    fn fetch_release(
        &self,
        channel: RuntimeReleaseChannel,
        target: &str,
    ) -> Result<SignedRuntimeRelease, RuntimeUpdatePortError> {
        let manifest = &self.release.payload;
        if manifest.channel != channel || manifest.artifact.target != target {
            return Err(RuntimeUpdatePortError::Unavailable(
                "no cached release for the requested channel and target".into(),
            ));
        }
        Ok(self.release.clone())
    }
}

/// Loads a revalidated cached-release checker for an explicit daemon profile.
///
/// # Errors
/// Returns an error when enabled configuration has no cache, key, or valid signed metadata.
pub(crate) fn load(
    enabled: bool,
    cache_path: Option<&Path>,
    trust_path: Option<&Path>,
    keys: &[String],
    now_unix_seconds: u64,
) -> Result<Option<DaemonRuntimeUpdateChecks>, String> {
    if !enabled {
        if cache_path.is_some() || trust_path.is_some() || !keys.is_empty() {
            return Err(
                "runtime release cache and trust settings require --runtime-update-check-authority"
                    .into(),
            );
        }
        return Ok(None);
    }
    let trusted = load_trusted(cache_path, trust_path, keys, now_unix_seconds)?;
    Ok(Some(RuntimeUpdateCheckService::new(
        trusted.source,
        trusted.trust,
        trusted.context,
        RuntimeUpdateCheckAuthority::CachedReadOnly,
    )))
}

/// Loads one cached signed release for a separately approved local authority action.
///
/// No network source is accepted here: a caller must have already placed the signed cache
/// through its own authenticated distribution path.
pub(crate) fn load_trusted(
    cache_path: Option<&Path>,
    trust_path: Option<&Path>,
    keys: &[String],
    now_unix_seconds: u64,
) -> Result<TrustedRuntimeRelease, String> {
    let cache_path = cache_path.ok_or("runtime update requires --runtime-release-cache")?;
    if trust_path.is_none() && keys.is_empty() {
        return Err("runtime update requires a trust document or --runtime-release-key".into());
    }
    let trust = RuntimeReleaseTrust::new(load_keys(trust_path, keys)?);
    let cached = CachedRuntimeRelease::load(cache_path, &trust, now_unix_seconds)
        .map_err(|error| format!("runtime release cache is not trusted: {error}"))?;
    Ok(TrustedRuntimeRelease {
        source: CachedReleaseSource {
            release: cached.release().clone(),
        },
        trust,
        context: RuntimeUpdateCheckContext {
            current_version: package_version(),
            target: platform_target()?,
            protocol: PROTOCOL_MAX,
            schema: gent_store::CURRENT_SCHEMA_VERSION,
            app_version: package_version(),
            selected_cohort: true,
        },
    })
}

fn load_keys(
    trust_path: Option<&Path>,
    explicit: &[String],
) -> Result<BTreeMap<String, VerifyingKey>, String> {
    let mut parsed = parse_keys(explicit)?;
    if let Some(path) = trust_path {
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "runtime release trust document is unavailable")?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("runtime release trust document must be a real file".into());
        }
        let bytes = fs::read(path).map_err(|_| "runtime release trust document is unreadable")?;
        for (key_id, key) in parse_trust_document(&bytes)
            .map_err(|_| "runtime release trust document has an unsupported shape")?
        {
            if parsed.insert(key_id, key).is_some() {
                return Err("runtime release trust repeats a key id".into());
            }
        }
    }
    Ok(parsed)
}

/// Reads one strict trust file without accepting command-line key additions.
///
/// # Errors
/// Returns an error when the file is unavailable, symlinked, or invalid.
pub(crate) fn load_keys_from_file(path: &Path) -> Result<BTreeMap<String, VerifyingKey>, String> {
    load_keys(Some(path), &[])
}

fn parse_keys(values: &[String]) -> Result<BTreeMap<String, VerifyingKey>, String> {
    let mut parsed = BTreeMap::new();
    for value in values {
        let (key_id, encoded) = value
            .split_once(':')
            .ok_or("runtime release key must be key-id:lowercase-hex")?;
        if key_id.is_empty()
            || encoded.len() != 64
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("runtime release key must be key-id:lowercase-hex".into());
        }
        let bytes = hex::decode(encoded).map_err(|_| "runtime release key is not hex")?;
        let key = VerifyingKey::from_bytes(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| "runtime release key must be 32 bytes")?,
        )
        .map_err(|_| "runtime release key is invalid")?;
        if parsed.insert(key_id.to_owned(), key).is_some() {
            return Err("runtime release trust repeats a key id".into());
        }
    }
    Ok(parsed)
}

pub(crate) fn package_version() -> RuntimeVersion {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let mut parse = || {
        parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("gentd package version must be numeric")
    };
    RuntimeVersion {
        major: parse(),
        minor: parse(),
        patch: parse(),
    }
}

pub(crate) fn platform_target() -> Result<String, String> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => Ok("aarch64-apple-darwin".into()),
        ("x86_64", "macos") => Ok("x86_64-apple-darwin".into()),
        ("x86_64", "linux") => Ok("x86_64-unknown-linux-gnu".into()),
        ("x86_64", "windows") => Ok("x86_64-pc-windows-msvc".into()),
        (arch, os) => Err(format!("runtime update checks do not support {arch}-{os}")),
    }
}

#[cfg(test)]
#[path = "runtime_update_config_tests.rs"]
mod tests;
