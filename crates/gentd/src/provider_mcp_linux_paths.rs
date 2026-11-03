//! Native host directory capabilities. A retained FD, not a pathname lock.
use super::mcp::Error;
use std::{
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};

pub(super) fn pin_data(
    path: &Path,
    workspace: &Path,
    worker: &Path,
) -> Result<(File, PathBuf), Error> {
    if !path.is_absolute() || path.starts_with(workspace) || path.starts_with(worker) {
        return Err(Error::Denied);
    }
    // Do not let canonicalization erase an agent-writable lexical traversal.
    for component in path.ancestors() {
        if std::fs::symlink_metadata(component)
            .map_err(|_| Error::Denied)?
            .file_type()
            .is_symlink()
        {
            return Err(Error::Denied);
        }
    }
    let canonical = path.canonicalize().map_err(|_| Error::Denied)?;
    if canonical != path || workspace.starts_with(&canonical) || worker == canonical {
        return Err(Error::Denied);
    }
    let before = std::fs::metadata(path).map_err(|_| Error::Denied)?;
    let file = File::open(path).map_err(|_| Error::Denied)?;
    let actual = file.metadata().map_err(|_| Error::Denied)?;
    let after = std::fs::metadata(path).map_err(|_| Error::Denied)?;
    if !actual.is_dir()
        || (before.dev(), before.ino()) != (actual.dev(), actual.ino())
        || (after.dev(), after.ino()) != (actual.dev(), actual.ino())
    {
        return Err(Error::Denied);
    }
    // Linux CLI uses data_dir/gentd.sock directly, without canonicalizing it or
    // hashing its spelling. The owning actor retains this descriptor through
    // ScopeKnown, including partial-start Unknown quarantine.
    let capability = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        file.as_raw_fd()
    ));
    Ok((file, capability))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{
        fs::symlink,
        net::{UnixListener, UnixStream},
    };
    #[test]
    fn native_data_fd_survives_path_replacement_without_redirecting_ipc() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let original = UnixListener::bind(data.join("gentd.sock")).unwrap();
        original.set_nonblocking(true).unwrap();
        let (pin, cap) = pin_data(&data, &workspace, &root.path().join("worker")).unwrap();
        std::fs::rename(&data, root.path().join("old")).unwrap();
        std::fs::create_dir(&data).unwrap();
        let replacement = UnixListener::bind(data.join("gentd.sock")).unwrap();
        replacement.set_nonblocking(true).unwrap();
        let _client = UnixStream::connect(cap.join("gentd.sock")).unwrap();
        let _accepted = original.accept().unwrap();
        assert!(replacement.accept().is_err());
        drop(pin);
        assert!(UnixStream::connect(cap.join("gentd.sock")).is_err());
    }
    #[test]
    fn native_data_rejects_agent_symlink_and_lexical_escape() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let link = workspace.join("link");
        symlink(&data, &link).unwrap();
        assert!(pin_data(&link, &workspace, &root.path().join("worker")).is_err());
        assert!(
            pin_data(
                &workspace.join("../data"),
                &workspace,
                &root.path().join("worker")
            )
            .is_err()
        );
    }
}
