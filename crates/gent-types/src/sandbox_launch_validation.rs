use crate::{RunVersionLock, SandboxLaunchContractError};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

pub(super) fn trusted_roots(
    roots: &[PathBuf],
    workspace_root: &Path,
) -> Result<Vec<PathBuf>, SandboxLaunchContractError> {
    let mut roots = roots
        .iter()
        .map(|root| trusted_root(root))
        .collect::<Result<Vec<_>, _>>()?;
    if roots.iter().any(|root| !root.starts_with(workspace_root)) {
        return Err(SandboxLaunchContractError::InvalidRoots);
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

pub(super) fn trusted_root(root: &Path) -> Result<PathBuf, SandboxLaunchContractError> {
    if !root.is_absolute()
        || root.parent().is_none()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(SandboxLaunchContractError::InvalidRoots);
    }
    Ok(root.to_path_buf())
}

pub(super) fn allowed_environment(
    mut values: Vec<String>,
) -> Result<Vec<String>, SandboxLaunchContractError> {
    const ALLOWED: &[&str] = &[
        "COLORTERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "NO_COLOR",
        "TERM",
        "TMP",
        "TMPDIR",
    ];
    if values
        .iter()
        .any(|value| !ALLOWED.contains(&value.as_str()))
    {
        return Err(SandboxLaunchContractError::InvalidEnvironment);
    }
    values.sort();
    values.dedup();
    Ok(values)
}

pub(super) fn valid_lock(lock: &RunVersionLock) -> bool {
    !lock.provider.is_empty()
        && !lock.canonical_path.is_empty()
        && !lock.file_identity.is_empty()
        && !lock.version.is_empty()
        && !lock.compatibility_entry.is_empty()
        && validate_digest(&lock.digest_sha256).is_ok()
}

pub(super) fn validate_digest(value: &str) -> Result<(), SandboxLaunchContractError> {
    (value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
    .then_some(())
    .ok_or(SandboxLaunchContractError::InvalidEgressPolicy)
}

pub(super) fn write_field(hasher: &mut Sha256, name: &str, value: &str) {
    hasher.update(name.len().to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}
