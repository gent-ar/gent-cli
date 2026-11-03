use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::provider_executables::ManagedProviderRuntime;

// Three complete finite PowerShell closures share one pinned manifest.
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ResourceRoot {
    NodeRuntime,
    Package,
    GitRuntime,
    PowerShellRuntime,
    PowerShellLauncher,
    OsGlobalizationData,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResourceFile {
    pub(crate) root: ResourceRoot,
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderResources {
    pub(crate) provider: String,
    pub(crate) platform: String,
    pub(crate) package_name: Option<String>,
    pub(crate) package_version: Option<String>,
    pub(crate) package_integrity: Option<String>,
    pub(crate) node_sha256: Option<String>,
    pub(crate) executable_sha256: Option<String>,
    pub(crate) credential_contract: String,
    pub(crate) files: Vec<ResourceFile>,
    pub(crate) os_libraries: Vec<String>,
}

pub(crate) const COMMAND_SHIMS: [&str; 3] = ["bash.exe", "pwsh.exe", "cmd.exe"];

pub(crate) fn require_command_shims(contract: &ProviderResources) -> Result<(), String> {
    let digests: Vec<_> = COMMAND_SHIMS
        .iter()
        .map(|name| {
            contract
                .files
                .iter()
                .find(|file| file.root == ResourceRoot::PowerShellLauncher && file.path == *name)
                .map(|file| file.sha256.as_str())
        })
        .collect();
    match digests.first().copied().flatten() {
        Some(digest) if digests.iter().all(|other| *other == Some(digest)) => Ok(()),
        _ => {
            Err("the reviewed command shim closure is incomplete; repair the runtime bundle".into())
        }
    }
}

/// The managed MSYS runtime must use the reviewed per-run temporary and drive mounts.
pub(crate) fn require_git_mounts(contract: &ProviderResources) -> Result<(), String> {
    const FSTAB_SHA256: &str = "af2184544d99dd656f4e5e119d80f2cb15f03c29d3d8467da1df5aa330508f4a";
    if contract.files.iter().any(|file| {
        file.root == ResourceRoot::GitRuntime
            && file.path == "etc/fstab"
            && file.sha256 == FSTAB_SHA256
    }) {
        Ok(())
    } else {
        Err("managed Git mount configuration is missing or unreviewed".into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u16,
    providers: Vec<ProviderResources>,
}

pub(crate) fn load(root: &Path, provider: &str) -> Result<(ProviderResources, File), String> {
    let pin = option_env!("GENT_WINDOWS_PROVIDER_RESOURCES_SHA256")
        .ok_or("reviewed managed provider resource manifest is unavailable")?;
    if !digest(pin) {
        return Err("invalid managed provider resource manifest pin".into());
    }
    let path = root.join("provider-resources.json");
    if path.canonicalize().ok().as_ref() != Some(&path) {
        return Err("managed resource manifest is missing or redirected".into());
    }
    let mut file = open_immutable(&path)?;
    if file
        .metadata()
        .map_err(|_| "managed resource manifest unavailable")?
        .len()
        > MAX_BYTES
    {
        return Err("managed resource manifest exceeds its bound".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "managed resource manifest unavailable")?;
    if bytes.len() as u64 > MAX_BYTES || hex::encode(Sha256::digest(&bytes)) != pin {
        return Err("managed resource manifest does not match this daemon build".into());
    }
    Ok((parse(&bytes, provider)?, file))
}

pub(crate) fn parse(bytes: &[u8], provider: &str) -> Result<ProviderResources, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("managed resource manifest exceeds its bound".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|_| "managed resource manifest is malformed")?;
    if manifest.version != 1 || manifest.providers.is_empty() || manifest.providers.len() > 3 {
        return Err("managed resource manifest is unsupported".into());
    }
    let mut providers = BTreeSet::new();
    for entry in &manifest.providers {
        let Some(provider_contract) =
            gent_types::AgentChatProvider::from_contained_id(&entry.provider)
                .map(gent_types::AgentChatProvider::contained_contract)
        else {
            return Err("managed provider resource contract is unsupported".into());
        };
        if !providers.insert(entry.provider.clone())
            || !matches!(entry.platform.as_str(), "win32-x64" | "win32-arm64")
            || entry.files.is_empty()
            || entry.files.len() > 1024
            || entry.os_libraries.is_empty()
            || entry.os_libraries.len() > 128
            || !provider_contract
                .credential_contracts
                .contains(&entry.credential_contract.as_str())
        {
            return Err("managed provider resource contract is unsupported".into());
        }
        let managed_package =
            provider_contract.route == gent_types::ContainedProviderRoute::ManagedPackage;
        if managed_package
            && (!entry.node_sha256.as_deref().is_some_and(digest)
                || entry.package_name.as_ref().is_none_or(String::is_empty)
                || entry.package_version.as_ref().is_none_or(String::is_empty)
                || entry
                    .package_integrity
                    .as_ref()
                    .is_none_or(String::is_empty))
        {
            return Err("managed provider package identity is incomplete".into());
        }
        if !managed_package
            && (!entry.executable_sha256.as_deref().is_some_and(digest)
                || entry.node_sha256.is_some()
                || entry
                    .files
                    .iter()
                    .any(|file| file.root == ResourceRoot::NodeRuntime))
        {
            return Err("Claurst resource identity is incomplete".into());
        }
        let mut files = BTreeSet::new();
        for file in &entry.files {
            if !relative(&file.path)
                || (file.root == ResourceRoot::PowerShellLauncher
                    && !COMMAND_SHIMS.contains(&file.path.as_str()))
                || (file.root == ResourceRoot::OsGlobalizationData
                    && file.path != "Globalization/Sorting/sortdefault.nls")
                || !digest(&file.sha256)
                || !files.insert((file.root, file.path.to_ascii_lowercase()))
            {
                return Err("managed provider resource file is invalid or duplicated".into());
            }
        }
        let mut libraries = BTreeSet::new();
        for name in &entry.os_libraries {
            if !relative(name)
                || name.contains('/')
                || !name.to_ascii_lowercase().ends_with(".dll")
                || !libraries.insert(name.to_ascii_lowercase())
            {
                return Err("managed provider OS library name is invalid or duplicated".into());
            }
        }
    }
    let mut contract = manifest
        .providers
        .into_iter()
        .find(|entry| entry.provider == provider)
        .ok_or("managed provider resource contract is missing")?;
    for library in &mut contract.os_libraries {
        library.make_ascii_lowercase();
    }
    Ok(contract)
}

impl ProviderResources {
    pub(crate) fn require_managed(&self, runtime: &ManagedProviderRuntime) -> Result<(), String> {
        let package = runtime.package();
        if (
            self.provider.as_str(),
            self.package_name.as_deref(),
            self.package_version.as_deref(),
            self.package_integrity.as_deref(),
            self.node_sha256.as_deref(),
        ) != (
            runtime.executable().provider.as_str(),
            Some(package.package_name.as_str()),
            Some(package.version.as_str()),
            Some(package.integrity.as_str()),
            Some(runtime.node().node_digest_sha256()),
        ) || gent_adapters::provider_platform::host_platform()
            .is_none_or(|platform| platform.npm != self.platform)
            || self
                .executable_sha256
                .as_ref()
                .is_some_and(|digest| digest != &runtime.executable().digest_sha256)
        {
            return Err(
                "managed resource contract does not match the approved installation".into(),
            );
        }
        Ok(())
    }
}

pub(crate) fn open_immutable(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| "managed runtime file cannot be locked against replacement")?;
    if !file
        .metadata()
        .map_err(|_| "managed runtime file unavailable")?
        .is_file()
    {
        return Err("managed runtime resource must be a regular file".into());
    }
    Ok(file)
}

pub(crate) fn resource_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !self::relative(relative) {
        return Err("managed runtime resource path is invalid".into());
    }
    let path = relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part));
    let canonical = path
        .canonicalize()
        .map_err(|_| "managed runtime resource is missing or redirected")?;
    if !canonical.as_os_str().eq_ignore_ascii_case(path.as_os_str()) {
        return Err("managed runtime resource is missing or redirected".into());
    }
    Ok(canonical)
}

pub(crate) fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.split('/').all(|part| {
            !part.is_empty()
                && !matches!(part, "." | "..")
                && !part.ends_with('.')
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@')
                })
        })
}

pub(crate) fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
#[path = "windows_provider_manifest_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "windows_provider_manifest_bound_tests.rs"]
mod bound_tests;
