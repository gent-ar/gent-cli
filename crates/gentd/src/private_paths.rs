//! Unix-only private filesystem boundary for the local daemon.

use std::fs::{self, DirBuilder};
use std::io::{Error, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::net::UnixListener;

/// Creates or repairs the daemon-owned root so other local users cannot traverse it.
pub(super) fn prepare_data_dir(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "gent data directory must not be a symlink",
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "gent data directory is not a directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            DirBuilder::new().recursive(true).mode(0o700).create(path)?;
        }
        Err(error) => return Err(error),
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    private(path, "gent data directory")
}

/// Binds a user-only socket inside the already-private daemon directory.
pub(super) fn bind_socket(data_dir: &Path, socket: &Path) -> std::io::Result<UnixListener> {
    let socket = resolved_socket_path(data_dir, socket)?;
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::AddrInUse && stale_socket(&socket) => {
            fs::remove_file(&socket)?;
            UnixListener::bind(&socket)?
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .and_then(|()| private(&socket, "gent socket"))
    {
        drop(listener);
        let _ = fs::remove_file(socket);
        return Err(error);
    }
    Ok(listener)
}

fn stale_socket(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_err_and(|error| {
        matches!(
            error.kind(),
            ErrorKind::ConnectionRefused | ErrorKind::NotFound
        )
    })
}

fn resolved_socket_path(data_dir: &Path, socket: &Path) -> std::io::Result<PathBuf> {
    let file_name = socket.file_name().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "gent socket path must name a socket",
        )
    })?;
    let parent = socket.parent().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "gent socket path must have a parent directory",
        )
    })?;
    let data_dir = fs::canonicalize(data_dir)?;
    let parent = fs::canonicalize(parent)?;
    let socket = parent.join(file_name);
    if parent == data_dir {
        Ok(socket)
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            "gent socket must be inside the private data directory",
        ))
    }
}

fn private(path: &Path, kind: &str) -> std::io::Result<()> {
    if fs::metadata(path)?.permissions().mode().trailing_zeros() >= 6 {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::PermissionDenied,
            format!("{kind} must be owner-only"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::{bind_socket, prepare_data_dir};

    #[test]
    fn data_directory_is_private_and_rejects_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let created = directory.path().join("created");
        prepare_data_dir(&created).unwrap();
        assert_eq!(
            fs::metadata(created).unwrap().permissions().mode() & 0o777,
            0o700
        );

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        prepare_data_dir(directory.path()).unwrap();
        assert_eq!(
            fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );

        let link = directory.path().join("link");
        std::os::unix::fs::symlink(directory.path(), &link).unwrap();
        assert!(prepare_data_dir(&link).is_err());
    }

    #[tokio::test]
    async fn socket_is_private_and_cannot_escape_data_directory() {
        let directory = tempfile::tempdir().unwrap();
        prepare_data_dir(directory.path()).unwrap();
        let socket = directory.path().join("gentd.sock");
        let _listener = bind_socket(directory.path(), &socket).unwrap();
        assert_eq!(
            fs::metadata(socket).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            bind_socket(
                directory.path(),
                &directory.path().join("nested/gentd.sock")
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn stale_socket_is_replaced_after_its_owner_stops() {
        let directory = tempfile::tempdir().unwrap();
        prepare_data_dir(directory.path()).unwrap();
        let socket = directory.path().join("gentd.sock");
        let listener = bind_socket(directory.path(), &socket).unwrap();
        drop(listener);
        assert!(bind_socket(directory.path(), &socket).is_ok());
    }
}
