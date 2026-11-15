use std::{
    fs::{self, File},
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use gent_ports::ApprovedPackageInstall;
use gent_types::ProviderInstallProgress;
use sha2::{Digest, Sha512};
use tempfile::Builder;

use crate::installer::{
    InstallerError, InstallerInvocation, NpmGlobalPrefix, ProviderInstallError,
};

#[path = "npm_registry_download.rs"]
mod registry;

/// Installs public providers only after verifying the packed tarball's SRI.
#[derive(Clone, Debug, Default)]
pub struct VerifiedNpmInstaller;

impl VerifiedNpmInstaller {
    /// Fetches the exact package, verifies its tarball, then installs that file.
    pub fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        self.install_with_progress(npm, package, &|_| {})
            .map_err(Into::into)
    }

    pub fn install_with_progress(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
        progress: &dyn Fn(ProviderInstallProgress),
    ) -> Result<(), ProviderInstallError> {
        npm.validate_prefix()?;
        npm.recheck()
            .map_err(|error| InstallerError::Runtime(error.to_string()))?;
        let staging = staging_directory(npm.prefix())?;
        let archive = staging.path().join("provider.tgz");
        if let Err(error) = registry::download(package, &archive, progress) {
            if error == ProviderInstallError::DownloadFailed {
                progress(ProviderInstallProgress::DownloadFailed);
            }
            return Err(error);
        }
        progress(ProviderInstallProgress::VerifyingArchive);
        let archive = canonical_archive(staging.path(), &archive)?;
        verify_integrity(&archive, &package.integrity)?;
        NpmGlobalPrefix::validate_filesystem_path(&archive)?;
        let invocation = npm.install_archive(&archive);
        let consumer_path = PathBuf::from(crate::installer::npm_archive_path(&archive));
        verify_archive_argument(&archive, &consumer_path)?;
        progress(ProviderInstallProgress::Installing);
        run(npm, &invocation).map_err(Into::into)
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

fn canonical_archive(staging: &Path, archive: &Path) -> Result<PathBuf, InstallerError> {
    let metadata = fs::symlink_metadata(archive).map_err(|error| io_error(&error))?;
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

// Recheck the consumer spelling after integrity verification: removing a Windows
// verbatim prefix must not make npm open a different file (for example, a name
// whose trailing dot has different ordinary Win32 semantics).
fn verify_archive_argument(
    verified_archive: &Path,
    consumer_path: &Path,
) -> Result<(), InstallerError> {
    let consumer_archive = fs::canonicalize(consumer_path).map_err(|error| io_error(&error))?;
    if consumer_archive != verified_archive {
        return Err(InstallerError::InvalidArtifact);
    }
    Ok(())
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
    let mut file = BufReader::new(File::open(archive).map_err(|error| io_error(&error))?);
    let mut digest = Sha512::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| io_error(&error))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = digest.finalize();
    if actual.as_slice() == expected.as_slice() {
        Ok(())
    } else {
        Err(InstallerError::IntegrityMismatch)
    }
}

fn run(npm: &NpmGlobalPrefix, invocation: &InstallerInvocation) -> Result<(), InstallerError> {
    npm.recheck()
        .map_err(|error| InstallerError::Runtime(error.to_string()))?;
    let mut command = gent_types::hidden_command(&invocation.executable);
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
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;

    use base64::{Engine, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha512};

    #[cfg(unix)]
    use super::staging_directory;
    use super::verify_integrity;

    #[test]
    fn npm_consumer_argument_must_resolve_to_the_integrity_checked_archive() {
        let root = tempfile::tempdir().unwrap();
        let verified = root.path().join("verified.tgz");
        let other = root.path().join("other.tgz");
        fs::write(&verified, b"verified").unwrap();
        fs::write(&other, b"other").unwrap();
        let canonical = verified.canonicalize().unwrap();
        let consumer = std::path::PathBuf::from(crate::installer::npm_archive_path(&canonical));
        super::verify_archive_argument(&canonical, &consumer).unwrap();
        assert!(super::verify_archive_argument(&canonical, &other).is_err());
        assert!(
            super::verify_archive_argument(&canonical, &root.path().join("missing.tgz")).is_err()
        );
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
