use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ed25519_dalek::VerifyingKey;
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};

use self::published::{PublishedAuthorityError, PublishedAuthorityOutcome};
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
    packaged: PathBuf,
    published: PathBuf,
    root_keys: BTreeMap<String, VerifyingKey>,
    runtime: Option<AppNodeRuntimeLock>,
}

impl StandaloneAuthorityRelease {
    pub(crate) fn resolve(
        explicit: Option<PathBuf>,
        root_keys: &[String],
        data_dir: &Path,
    ) -> Result<Option<Self>, String> {
        let Some(authority) = authority_source(
            explicit,
            root_keys,
            packaged::PackagedAuthority::from_current_executable,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(Self::adopting(
            authority.release,
            data_dir,
            parse_keys(&authority.root_keys)?,
            AppNodeRuntimeLock::from_standalone_environment(data_dir).ok(),
        )))
    }

    #[cfg(test)]
    pub(crate) fn configured(
        path: PathBuf,
        key_specs: &[String],
        runtime: AppNodeRuntimeLock,
    ) -> Result<Self, String> {
        let data_dir = path.parent().unwrap_or(&path).to_path_buf();
        Ok(Self::adopting(
            path,
            &data_dir,
            parse_keys(key_specs)?,
            Some(runtime),
        ))
    }

    fn adopting(
        packaged: PathBuf,
        data_dir: &Path,
        root_keys: BTreeMap<String, VerifyingKey>,
        runtime: Option<AppNodeRuntimeLock>,
    ) -> Self {
        let directory = data_dir.join("provider-authority");
        let mut release = Self {
            path: packaged.clone(),
            packaged,
            published: directory.join("published.json"),
            root_keys,
            runtime,
        };
        let adopted = directory.join("ordinary-authority.json");
        if release.expiry(&release.published) > release.expiry(&adopted)
            && let Err(error) = std::fs::rename(&release.published, &adopted)
        {
            eprintln!("gentd could not adopt the published provider authority: {error}");
        }
        if release.expiry(&adopted) > release.expiry(&release.packaged) {
            release.path = adopted;
        }
        release
    }

    fn expiry(&self, path: &Path) -> Option<u64> {
        let runtime = self.runtime.as_ref()?;
        let now = crate::startup::unix_seconds();
        SignedOrdinaryAuthorityRelease::load_bound(path, &self.root_keys, runtime, now)
            .ok()
            .map(|release| release.expires_at_unix_seconds())
    }

    pub(crate) fn load(
        &self,
        now: u64,
    ) -> Result<VerifiedOrdinaryAuthorityRelease, OrdinaryAuthorityReleaseError> {
        self.load_from(&self.path, now)
    }

    pub(crate) fn load_packaged(
        &self,
        now: u64,
    ) -> Result<VerifiedOrdinaryAuthorityRelease, OrdinaryAuthorityReleaseError> {
        self.load_from(&self.packaged, now)
    }

    fn load_from(
        &self,
        path: &Path,
        now: u64,
    ) -> Result<VerifiedOrdinaryAuthorityRelease, OrdinaryAuthorityReleaseError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or(OrdinaryAuthorityReleaseError::RuntimeUnverified)?;
        SignedOrdinaryAuthorityRelease::load_bound(path, &self.root_keys, runtime, now)
    }

    pub(crate) fn stage_published(
        &self,
        document: &[u8],
        now: u64,
    ) -> Result<PublishedAuthorityOutcome, PublishedAuthorityError> {
        let directory = self.published.parent().unwrap_or(&self.published);
        std::fs::create_dir_all(directory).map_err(PublishedAuthorityError::Store)?;
        let mut staged =
            tempfile::NamedTempFile::new_in(directory).map_err(PublishedAuthorityError::Store)?;
        std::io::Write::write_all(&mut staged, document).map_err(PublishedAuthorityError::Store)?;
        let offered = self
            .load_from(staged.path(), now)?
            .expires_at_unix_seconds();
        if self.expiry(&self.path).max(self.expiry(&self.published)) >= Some(offered) {
            return Ok(PublishedAuthorityOutcome::Current);
        }
        staged
            .persist(&self.published)
            .map_err(|error| PublishedAuthorityError::Store(error.error))?;
        Ok(PublishedAuthorityOutcome::Staged)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn linux_deployment(
        &self,
        now: u64,
        node: &gent_drivers::node_runtime_lock::NodeRuntimeLock,
    ) -> Result<crate::linux_provider_deployment::VerifiedLinuxDeployment, String> {
        let ordinary = self
            .load_packaged(now)
            .map_err(|_| "Ordinary release authority is unavailable")?;
        node.recheck()
            .map_err(|_| "Verified packaged Node runtime changed")?;
        let root = crate::linux_provider_deployment::resource_root_for_node(node.node_path())
            .map_err(str::to_owned)?;
        if self.packaged != root.join("authority/ordinary-authority.json") {
            return Err(
                "Linux ordinary authority must belong to the verified packaged Node release".into(),
            );
        }
        crate::linux_provider_deployment::VerifiedLinuxDeployment::load(
            &root.join("authority/linux-sandbox.json"),
            &root,
            &self.root_keys,
            ordinary.artifact_digest_sha256(),
            now,
        )
        .map_err(str::to_owned)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn runtime(&self) -> Option<&AppNodeRuntimeLock> {
        self.runtime.as_ref()
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

#[path = "published_provider_authority.rs"]
pub(crate) mod published;

#[cfg(test)]
mod tests {
    use super::parse_keys;

    #[test]
    fn parses_one_bounded_release_root_key() {
        let keys = parse_keys(&[format!("root:{}", "01".repeat(32))]).unwrap();
        assert_eq!(keys.len(), 1);
    }
}
