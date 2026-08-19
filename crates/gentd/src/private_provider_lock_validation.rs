//! Immutable lock validation shared by private provisioning and its focused tests.

use std::path::Path;

use gent_protocol::DependencyProvider;

use crate::private_provider_provisioning::ProvisionedProviderLock;

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn valid_lock(
    lock: &ProvisionedProviderLock,
    provider: DependencyProvider,
    prefix: &Path,
) -> bool {
    let Ok(executable) = Path::new(&lock.run_lock.canonical_path).canonicalize() else {
        return false;
    };
    let Ok(prefix) = prefix.canonicalize() else {
        return false;
    };
    lock.run_lock.provider == provider.as_str()
        && executable.starts_with(prefix)
        && executable.is_file()
        && executable.display().to_string() == lock.run_lock.canonical_path
        && valid_version(&lock.run_lock.version)
        && valid_digest(&lock.run_lock.digest_sha256)
        && gent_drivers::lock::recheck(&lock.run_lock).is_ok()
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 512
        && version.trim() == version
        && !version.contains('\0')
}
