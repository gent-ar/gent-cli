//! Exclusive, receipt-bound attempt directories. Existing attempts are never reused.

use std::{fs, path::Path};

use gent_drivers::installer::NpmGlobalPrefix;
use gent_types::Command;

use super::PrivateProvisionError;
use crate::{
    node_runtime_lock::AppNodeRuntimeLock,
    standalone_provider_setup::{provider_generation_prefix, verified_generation_prefix},
};

pub(super) fn fresh(
    runtime: &AppNodeRuntimeLock,
    command: &Command,
) -> Result<NpmGlobalPrefix, PrivateProvisionError> {
    let existing = runtime.rechecked_npm_prefix()?;
    let fingerprint = command.receipt_fingerprint_sha256();
    let prefix = provider_generation_prefix(existing.prefix(), &fingerprint)
        .ok_or_else(|| failure("invalid accepted command installation identity"))?;
    let generation = prefix
        .parent()
        .ok_or_else(|| failure("missing generation"))?;
    let generations = generation
        .parent()
        .ok_or_else(|| failure("missing generations root"))?;
    let providers = generations
        .parent()
        .ok_or_else(|| failure("missing providers root"))?;
    fs::create_dir_all(providers).map_err(|error| failure(&error.to_string()))?;
    require_directory(providers)?;
    match fs::create_dir(generations) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(failure(&error.to_string())),
    }
    require_directory(generations)?;
    if generations
        .canonicalize()
        .map_err(|error| failure(&error.to_string()))?
        != providers
            .canonicalize()
            .map_err(|error| failure(&error.to_string()))?
            .join("generations")
    {
        return Err(failure(
            "generation root is not below the owned providers directory",
        ));
    }
    // Atomic creation fails even if the previous attempt left an empty directory.
    fs::create_dir(generation).map_err(|error| failure(&error.to_string()))?;
    fs::create_dir(&prefix).map_err(|error| failure(&error.to_string()))?;
    let prefix = verified_generation_prefix(existing.prefix(), &fingerprint)
        .ok_or_else(|| failure("fresh generation escaped its owned root"))?;
    Ok(NpmGlobalPrefix::new(runtime.rechecked_lock()?, prefix))
}

fn require_directory(path: &Path) -> Result<(), PrivateProvisionError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| failure(&error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(failure(
            "provider installation ancestor is not an ordinary directory",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(failure("provider installation ancestor is a reparse point"));
        }
    }
    Ok(())
}

fn failure(message: &str) -> PrivateProvisionError {
    PrivateProvisionError::Generation(message.into())
}
