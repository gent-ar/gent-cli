use gent_protocol::runtime_owner::RuntimeOwnerFrame;
use gent_types::HostEpoch;
use std::{path::PathBuf, sync::OnceLock};

#[derive(Clone, Debug)]
struct StartupIdentity {
    process_id: u32,
    executable_digest_sha256: String,
    canonical_executable_path: PathBuf,
    canonical_data_directory: PathBuf,
    boot_nonce: String,
}
static STARTUP_IDENTITY: OnceLock<StartupIdentity> = OnceLock::new();

pub(crate) fn initialize(held: &crate::host_lock::HostLock) -> Result<(), std::io::Error> {
    let identity = capture(held)?;
    #[cfg(test)]
    {
        let _ = STARTUP_IDENTITY.set(identity);
        Ok(())
    }
    #[cfg(not(test))]
    STARTUP_IDENTITY
        .set(identity)
        .map_err(|_| std::io::Error::other("runtime owner identity was already captured"))
}

fn capture(held: &crate::host_lock::HostLock) -> Result<StartupIdentity, std::io::Error> {
    let canonical_executable_path = std::env::current_exe()?.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&canonical_executable_path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "runtime executable identity is unavailable",
        ));
    }
    let executable_digest_sha256 = crate::api::executable_digest()
        .ok_or_else(|| std::io::Error::other("runtime executable digest is unavailable"))?
        .to_owned();
    Ok(StartupIdentity {
        process_id: std::process::id(),
        executable_digest_sha256,
        canonical_executable_path,
        canonical_data_directory: held.canonical_data_directory().to_owned(),
        boot_nonce: uuid::Uuid::new_v4().to_string(),
    })
}

pub(crate) fn frame(request_id: String, epoch: HostEpoch) -> RuntimeOwnerFrame {
    frame_from(STARTUP_IDENTITY.get(), request_id, epoch)
}

fn frame_from(
    identity: Option<&StartupIdentity>,
    request_id: String,
    epoch: HostEpoch,
) -> RuntimeOwnerFrame {
    match identity {
        Some(identity) => identity_frame(identity, request_id, epoch),
        None => RuntimeOwnerFrame::RuntimeOwnerIdentityUnavailable { request_id },
    }
}

fn identity_frame(
    identity: &StartupIdentity,
    request_id: String,
    epoch: HostEpoch,
) -> RuntimeOwnerFrame {
    RuntimeOwnerFrame::RuntimeOwnerIdentityState {
        request_id,
        host_epoch: epoch,
        process_id: identity.process_id,
        executable_digest_sha256: identity.executable_digest_sha256.clone(),
        canonical_executable_path: identity.canonical_executable_path.clone(),
        canonical_data_directory: identity.canonical_data_directory.clone(),
        boot_nonce: identity.boot_nonce.clone(),
    }
}

#[cfg(test)]
#[path = "runtime_owner_identity_tests.rs"]
mod tests;
