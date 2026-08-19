//! Integrity-checked npm tarball installation for a future approved host.
//!
//! This driver is deliberately dormant.  It never selects a package and it
//! never exposes npm output: callers must provide a signed exact artifact.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use gent_ports::ApprovedPackageInstall;
use sha2::{Digest, Sha512};
use tempfile::Builder;

use crate::installer::{InstallerError, InstallerInvocation, NpmGlobalPrefix};

/// Installs public providers only after verifying the packed tarball's SRI.
#[derive(Clone, Debug, Default)]
pub struct VerifiedNpmInstaller;

impl VerifiedNpmInstaller {
    /// Fetches the exact package, verifies its tarball, then installs that file.
    ///
    /// # Errors
    /// Returns an error without installing when npm, its JSON response, archive
    /// path, or signed SHA-512 SRI verification is invalid.
    pub fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        let staging = staging_directory(npm.prefix())?;
        let archive = pack(npm, package, staging.path())?;
        verify_integrity(&archive, &package.integrity)?;
        run(npm, &npm.install_archive(&archive))
    }
}

fn staging_directory(prefix: &Path) -> Result<tempfile::TempDir, InstallerError> {
    let root = prefix.join(".pack-staging");
    fs::create_dir_all(&root).map_err(|error| io_error(&error))?;
    require_private_directory(&root)?;
    let staging = Builder::new()
        .prefix("verified-")
        .tempdir_in(root)
        .map_err(|error| io_error(&error))?;
    require_private_directory(staging.path())?;
    Ok(staging)
}

fn pack(
    npm: &NpmGlobalPrefix,
    package: &ApprovedPackageInstall,
    staging: &Path,
) -> Result<PathBuf, InstallerError> {
    npm.recheck()
        .map_err(|error| InstallerError::Runtime(error.to_string()))?;
    let invocation = npm.pack(package, staging);
    let mut command = Command::new(&invocation.executable);
    command.args(&invocation.arguments);
    npm.configure_command(&mut command)?;
    let output = command
        .output()
        .map_err(|error| InstallerError::Launch(error.to_string()))?;
    if !output.status.success() {
        return Err(InstallerError::Failed(output.status.to_string()));
    }
    let filename = pack_filename(&output.stdout)?;
    let archive = staging.join(filename);
    let metadata = fs::symlink_metadata(&archive).map_err(|error| io_error(&error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(InstallerError::InvalidArtifact);
    }
    let staging = fs::canonicalize(staging).map_err(|error| io_error(&error))?;
    let archive = fs::canonicalize(archive).map_err(|error| io_error(&error))?;
    if !archive.starts_with(&staging) || archive == staging {
        return Err(InstallerError::InvalidArtifact);
    }
    Ok(archive)
}

fn pack_filename(output: &[u8]) -> Result<String, InstallerError> {
    let value: serde_json::Value =
        serde_json::from_slice(output).map_err(|_| InstallerError::PackOutput)?;
    let items = value
        .as_array()
        .filter(|items| items.len() == 1)
        .ok_or(InstallerError::PackOutput)?;
    let filename = items
        .first()
        .and_then(|item| item.get("filename"))
        .and_then(serde_json::Value::as_str)
        .ok_or(InstallerError::PackOutput)?
        .to_owned();
    let path = Path::new(&filename);
    if path.file_name().and_then(|name| name.to_str()) != Some(&filename)
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tgz"))
    {
        return Err(InstallerError::InvalidArtifact);
    }
    Ok(filename)
}

fn require_private_directory(path: &Path) -> Result<(), InstallerError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(&error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(InstallerError::InvalidArtifact);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| io_error(&error))?;
        if fs::metadata(path)
            .map_err(|error| io_error(&error))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(InstallerError::InvalidArtifact);
        }
    }
    Ok(())
}

fn verify_integrity(archive: &Path, sri: &str) -> Result<(), InstallerError> {
    let expected = sri
        .strip_prefix("sha512-")
        .and_then(|value| STANDARD.decode(value).ok())
        .filter(|digest| digest.len() == 64)
        .ok_or(InstallerError::InvalidIntegrity)?;
    let actual = Sha512::digest(fs::read(archive).map_err(|error| io_error(&error))?);
    if actual.as_slice() == expected.as_slice() {
        Ok(())
    } else {
        Err(InstallerError::IntegrityMismatch)
    }
}

fn run(npm: &NpmGlobalPrefix, invocation: &InstallerInvocation) -> Result<(), InstallerError> {
    npm.recheck()
        .map_err(|error| InstallerError::Runtime(error.to_string()))?;
    let mut command = Command::new(&invocation.executable);
    command.args(&invocation.arguments);
    npm.configure_command(&mut command)?;
    let status = command
        .status()
        .map_err(|error| InstallerError::Launch(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(InstallerError::Failed(status.to_string()))
    }
}

fn io_error(error: &std::io::Error) -> InstallerError {
    InstallerError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use base64::{Engine, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha512};

    use super::{pack_filename, staging_directory, verify_integrity};

    #[test]
    fn accepts_only_a_single_safe_tarball_filename() {
        assert_eq!(
            pack_filename(br#"[{"filename":"codex-1.0.0.tgz"}]"#).unwrap(),
            "codex-1.0.0.tgz"
        );
        assert!(pack_filename(br#"[{"filename":"one.tgz"},{"filename":"two.tgz"}]"#).is_err());
        assert!(pack_filename(br#"[{"filename":"../outside.tgz"}]"#).is_err());
        assert!(pack_filename(br#"[{"filename":"archive.zip"}]"#).is_err());
    }

    #[test]
    fn compares_actual_tarball_bytes_to_sha512_sri() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("provider.tgz");
        fs::write(&archive, b"trusted tarball").unwrap();
        let sri = format!(
            "sha512-{}",
            STANDARD.encode(Sha512::digest(b"trusted tarball"))
        );
        verify_integrity(&archive, &sri).unwrap();
        fs::write(&archive, b"substituted tarball").unwrap();
        assert!(verify_integrity(&archive, &sri).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn staging_root_is_private_and_refuses_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        let staging = staging_directory(&prefix).unwrap();
        let mode = fs::metadata(prefix.join(".pack-staging"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
        drop(staging);
        fs::remove_dir_all(prefix.join(".pack-staging")).unwrap();
        fs::write(root.path().join("not-a-directory"), b"x").unwrap();
        symlink(
            root.path().join("not-a-directory"),
            prefix.join(".pack-staging"),
        )
        .unwrap();
        assert!(staging_directory(Path::new(&prefix)).is_err());
    }
}
