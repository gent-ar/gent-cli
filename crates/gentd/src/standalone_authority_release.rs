use std::{collections::BTreeMap, path::PathBuf};

use ed25519_dalek::VerifyingKey;
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};

use crate::{
    node_runtime_lock::AppNodeRuntimeLock,
    ordinary_authority_release::{
        OrdinaryAuthorityReleaseError, SignedOrdinaryAuthorityRelease,
        VerifiedOrdinaryAuthorityRelease,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneAuthorityRelease {
    path: PathBuf,
    root_keys: BTreeMap<String, VerifyingKey>,
    runtime: AppNodeRuntimeLock,
}

impl StandaloneAuthorityRelease {
    pub(crate) fn resolve(
        explicit: Option<PathBuf>,
        root_keys: &[String],
        data_dir: &std::path::Path,
    ) -> Result<Option<Self>, String> {
        let Some(authority) = authority_source(
            explicit,
            root_keys,
            packaged::PackagedAuthority::from_current_executable,
        )?
        else {
            return Ok(None);
        };
        let runtime = AppNodeRuntimeLock::from_standalone_environment(data_dir)
            .map_err(|error| error.to_string())?;
        Self::configured(authority.release, &authority.root_keys, runtime).map(Some)
    }

    pub(crate) fn configured(
        path: PathBuf,
        key_specs: &[String],
        runtime: AppNodeRuntimeLock,
    ) -> Result<Self, String> {
        Ok(Self {
            path,
            root_keys: parse_keys(key_specs)?,
            runtime,
        })
    }

    pub(crate) fn load(
        &self,
        now: u64,
    ) -> Result<VerifiedOrdinaryAuthorityRelease, OrdinaryAuthorityReleaseError> {
        SignedOrdinaryAuthorityRelease::load_bound(&self.path, &self.root_keys, &self.runtime, now)
    }

    pub(crate) fn runtime(&self) -> &AppNodeRuntimeLock {
        &self.runtime
    }

    pub(crate) fn provision_config(
        &self,
    ) -> crate::private_provider_provisioning::ReleaseAuthorityConfig {
        crate::private_provider_provisioning::ReleaseAuthorityConfig {
            path: self.path.clone(),
            root_keys: self.root_keys.clone(),
        }
    }
}

impl PackageInstallPolicy for StandaloneAuthorityRelease {
    fn approved_package(
        &self,
        provider: &str,
        now_unix_seconds: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        let release =
            self.load(now_unix_seconds)
                .map_err(|error| PackageInstallPolicyError::Rejected {
                    provider: provider.into(),
                    reason: error.to_string(),
                })?;
        if !release.authorizes_provider(provider) {
            return Err(PackageInstallPolicyError::Unavailable {
                provider: provider.into(),
            });
        }
        release
            .package_policy()
            .approved_package(provider, now_unix_seconds)
    }
}

pub(crate) fn authority_source(
    explicit: Option<PathBuf>,
    root_keys: &[String],
    packaged: impl FnOnce() -> Result<Option<packaged::PackagedAuthority>, String>,
) -> Result<Option<packaged::PackagedAuthority>, String> {
    match explicit {
        Some(release) => Ok(Some(packaged::PackagedAuthority {
            release,
            root_keys: root_keys.to_vec(),
        })),
        None => packaged(),
    }
}

fn parse_keys(values: &[String]) -> Result<BTreeMap<String, VerifyingKey>, String> {
    if values.is_empty() || values.len() > 8 {
        return Err("standalone authority release requires a bounded root key set".into());
    }
    let mut keys = BTreeMap::new();
    for value in values {
        let (id, encoded) = value
            .split_once(':')
            .ok_or("invalid standalone authority release key")?;
        let bytes = hex::decode(encoded)
            .map_err(|_| "invalid standalone authority release key".to_owned())?;
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || encoded.len() != 64
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err("invalid standalone authority release key".into());
        }
        let key = VerifyingKey::from_bytes(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid standalone authority release key")?,
        )
        .map_err(|_| "invalid standalone authority release key")?;
        if keys.insert(id.into(), key).is_some() {
            return Err("duplicate standalone authority release key".into());
        }
    }
    Ok(keys)
}

#[path = "packaged_authority.rs"]
pub(crate) mod packaged;

#[cfg(test)]
mod tests {
    use super::parse_keys;

    #[test]
    fn parses_one_bounded_release_root_key() {
        let keys = parse_keys(&[format!("root:{}", "01".repeat(32))]).unwrap();
        assert_eq!(keys.len(), 1);
    }
}
