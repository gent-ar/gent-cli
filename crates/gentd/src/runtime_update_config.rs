//! Daemon composition for an explicitly configured, cached update report.
//!
//! This module is intentionally limited to loading already-verified metadata.
//! It does not fetch, persist, stage, health-check, or activate a runtime.

use std::{collections::BTreeMap, fs, path::Path};

use ed25519_dalek::VerifyingKey;
use gent_ports::runtime_update::{RuntimeReleaseSource, RuntimeUpdatePortError};
use gent_runtime::{
    CachedRuntimeRelease, RuntimeReleaseTrust, RuntimeUpdateCheckAuthority,
    RuntimeUpdateCheckContext, RuntimeUpdateCheckService,
};
use gent_types::{PROTOCOL_MAX, RuntimeReleaseChannel, RuntimeVersion, SignedRuntimeRelease};

/// Concrete daemon checker assembled only after cache and trust configuration validate.
pub(crate) type DaemonRuntimeUpdateChecks = RuntimeUpdateCheckService<CachedReleaseSource>;

/// Immutable source backed by a release cache validated at daemon startup.
#[derive(Clone, Debug)]
pub(crate) struct CachedReleaseSource {
    release: SignedRuntimeRelease,
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
    let cache_path = cache_path.ok_or("runtime update check requires --runtime-release-cache")?;
    let trust = RuntimeReleaseTrust::new(load_keys(trust_path, keys)?);
    if trust_path.is_none() && keys.is_empty() {
        return Err(
            "runtime update check requires a trust document or --runtime-release-key".into(),
        );
    }
    let cached = CachedRuntimeRelease::load(cache_path, &trust, now_unix_seconds)
        .map_err(|error| format!("runtime release cache is not trusted: {error}"))?;
    Ok(Some(RuntimeUpdateCheckService::new(
        CachedReleaseSource {
            release: cached.release().clone(),
        },
        trust,
        RuntimeUpdateCheckContext {
            current_version: package_version(),
            target: platform_target()?,
            protocol: PROTOCOL_MAX,
            schema: gent_store::CURRENT_SCHEMA_VERSION,
            app_version: package_version(),
            selected_cohort: true,
        },
        RuntimeUpdateCheckAuthority::CachedReadOnly,
    )))
}

fn load_keys(
    trust_path: Option<&Path>,
    explicit: &[String],
) -> Result<BTreeMap<String, VerifyingKey>, String> {
    let mut values = explicit.to_vec();
    if let Some(path) = trust_path {
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "runtime release trust document is unavailable")?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("runtime release trust document must be a real file".into());
        }
        let document: serde_json::Value = serde_json::from_slice(
            &fs::read(path).map_err(|_| "runtime release trust document is unreadable")?,
        )
        .map_err(|_| "runtime release trust document is invalid JSON")?;
        let Some(entries) = document.get("keys").and_then(serde_json::Value::as_array) else {
            return Err("runtime release trust document has an unsupported shape".into());
        };
        if document
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
        {
            return Err("runtime release trust document has an unsupported shape".into());
        }
        for entry in entries {
            let key_id = entry.get("keyId").and_then(serde_json::Value::as_str);
            let key = entry
                .get("publicKeyHex")
                .and_then(serde_json::Value::as_str);
            let (Some(key_id), Some(key)) = (key_id, key) else {
                return Err("runtime release trust document has an invalid key entry".into());
            };
            values.push(format!("{key_id}:{key}"));
        }
    }
    parse_keys(&values)
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

fn package_version() -> RuntimeVersion {
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

fn platform_target() -> Result<String, String> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => Ok("aarch64-apple-darwin".into()),
        ("x86_64", "macos") => Ok("x86_64-apple-darwin".into()),
        ("x86_64", "linux") => Ok("x86_64-unknown-linux-gnu".into()),
        ("x86_64", "windows") => Ok("x86_64-pc-windows-msvc".into()),
        (arch, os) => Err(format!("runtime update checks do not support {arch}-{os}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ed25519_dalek::{Signer, SigningKey};
    use gent_runtime::{CachedRuntimeRelease, RuntimeReleaseTrust};
    use gent_types::{
        RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
        RuntimeReleaseManifest, RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeVersion,
        SignedRuntimeRelease,
    };

    use super::{load, load_keys, parse_keys, platform_target};

    #[test]
    fn key_parser_fails_closed() {
        assert!(parse_keys(&["key:00".into()]).is_err());
        assert!(parse_keys(&[format!("key:{}", "A".repeat(64))]).is_err());
    }

    #[test]
    fn target_is_one_of_the_published_release_targets() {
        assert!(platform_target().is_ok());
    }

    #[test]
    fn enabled_check_requires_a_trusted_cache_and_revalidates_it() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let target = platform_target().unwrap();
        let payload = RuntimeReleaseManifest {
            manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
            release_version: RuntimeVersion {
                major: 9,
                minor: 0,
                patch: 0,
            },
            protocol_min: 1,
            protocol_max: gent_types::PROTOCOL_MAX,
            schema_min: 1,
            schema_max: gent_store::CURRENT_SCHEMA_VERSION,
            minimum_app_version: RuntimeVersion {
                major: 0,
                minor: 1,
                patch: 0,
            },
            channel: RuntimeReleaseChannel::Stable,
            rollout_percent: 100,
            expires_at_unix_seconds: 10,
            revoked: false,
            forward_only_schema: false,
            artifact: RuntimeReleaseArtifact {
                target,
                archive_name: "gent.tar.gz".into(),
                digest_sha256: "a".repeat(64),
                size_bytes: 1,
            },
        };
        let release = SignedRuntimeRelease {
            key_id: "release-1".into(),
            signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
            payload,
        };
        let trust =
            RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime-release.json");
        CachedRuntimeRelease::verify(release, &trust, 1)
            .unwrap()
            .store(&path, &trust, 1)
            .unwrap();
        let key_text = format!("release-1:{}", hex::encode(key.verifying_key().to_bytes()));
        let checks = load(true, Some(&path), None, &[key_text], 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            checks
                .check(
                    RuntimeUpdateCheckRequest {
                        channel: RuntimeReleaseChannel::Stable,
                    },
                    1,
                )
                .state,
            RuntimeUpdateCheckState::Available
        );
        assert!(load(true, Some(&path), None, &[], 1).is_err());
    }

    #[test]
    fn trust_document_is_strict_and_can_supply_the_only_key() {
        let key = SigningKey::from_bytes(&[8; 32]);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trust.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"schemaVersion":1,"keys":[{{"keyId":"release-1","publicKeyHex":"{}"}}]}}"#,
                hex::encode(key.verifying_key().to_bytes())
            ),
        )
        .unwrap();
        assert!(load_keys(Some(&path), &[]).is_ok());
        std::fs::write(&path, r#"{"schemaVersion":2,"keys":[]}"#).unwrap();
        assert!(load_keys(Some(&path), &[]).is_err());
    }
}
