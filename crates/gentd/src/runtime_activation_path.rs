//! Resolve an immutable generation through pinned, non-redirected components.
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};

pub(super) struct VerifiedDaemon {
    pub(super) path: PathBuf,
    pub(super) file: File,
    _directories: Vec<File>,
}

pub(super) fn open(root: &Path, generation: &str, name: &str) -> Result<VerifiedDaemon, String> {
    if !root.is_absolute() || !one_component(generation) || !one_component(name) {
        return Err("immutable daemon generation path is invalid".into());
    }
    let releases = root.join("releases");
    let directory = releases.join(generation);
    let mut directories = Vec::new();
    for path in [root, releases.as_path(), directory.as_path()] {
        directories.push(pin(path, true)?);
    }
    let exact = directory.join(name);
    let file = pin(&exact, false)?;
    // Windows package filesystem virtualization can map this generation below
    // a different physical root. Anchor to the pinned generation, not a string
    // formed by appending children to the logical store root.
    let physical_directory = directory.canonicalize().map_err(display)?;
    let physical_file = exact.canonicalize().map_err(display)?;
    if physical_file.parent() != Some(physical_directory.as_path())
        || physical_file.file_name() != Some(std::ffi::OsStr::new(name))
    {
        return Err("immutable daemon generation was redirected".into());
    }
    Ok(VerifiedDaemon {
        path: physical_file,
        file,
        _directories: directories,
    })
}

fn one_component(value: &str) -> bool {
    let mut parts = Path::new(value).components();
    matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

fn pin(path: &Path, directory: bool) -> Result<File, String> {
    // The metadata precheck also rejects Unix symlinks before File::open.
    let metadata = fs::symlink_metadata(path).map_err(display)?;
    if redirected(&metadata)
        || metadata.is_dir() != directory
        || (!directory && !metadata.is_file())
    {
        return Err("immutable daemon generation contains a redirected or invalid entry".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ, OPEN_REPARSE_POINT, and directory BACKUP_SEMANTICS.
        // Hold the actual objects against writes, removal and reparse replacement.
        options
            .share_mode(1)
            .custom_flags(0x0020_0000 | if directory { 0x0200_0000 } else { 0 });
    }
    let file = options.open(path).map_err(display)?;
    let metadata = file.metadata().map_err(display)?;
    if redirected(&metadata)
        || metadata.is_dir() != directory
        || (!directory && !metadata.is_file())
    {
        return Err("immutable daemon generation changed before it was pinned".into());
    }
    Ok(file)
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn root() -> PathBuf {
        let base = std::env::var_os("GENT_ACTIVATION_PATH_TEST_ROOT")
            .map_or_else(std::env::temp_dir, PathBuf::from);
        let root = base.join(format!(
            "activation-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("releases/generation")).unwrap();
        fs::write(root.join("releases/generation/gentd.exe"), b"owned-daemon").unwrap();
        root
    }

    #[test]
    fn ordinary_and_native_namespace_aliases_preserve_exact_bytes() {
        let root = root();
        let physical = root.canonicalize().unwrap();
        for input in [&root, &physical] {
            let mut verified = open(input, "generation", "gentd.exe").unwrap();
            let mut bytes = Vec::new();
            verified.file.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"owned-daemon");
            assert_eq!(
                verified.path.parent(),
                Some(
                    root.join("releases/generation")
                        .canonicalize()
                        .unwrap()
                        .as_path()
                )
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn pinned_generation_and_file_cannot_be_replaced_or_written() {
        let root = root();
        let path = root.join("releases/generation/gentd.exe");
        let verified = open(&root, "generation", "gentd.exe").unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert!(
            fs::rename(
                root.join("releases/generation"),
                root.join("releases/moved")
            )
            .is_err()
        );
        drop(verified);
        fs::write(&path, b"after-release").unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn path_components_cannot_escape_the_generation() {
        let root = root();
        for value in ["../generation", "generation/../generation", ".", "..", ""] {
            assert!(open(&root, value, "gentd.exe").is_err());
        }
        assert!(open(&root, "generation", "../gentd.exe").is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn actual_directory_redirection_is_rejected() {
        let root = root();
        let target = root.join("outside");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("gentd.exe"), b"outside-canary").unwrap();
        let link = root.join("releases/redirected");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target, &link)
            .expect("owned reparse setup requires the existing host developer-mode support");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open(&root, "redirected", "gentd.exe").is_err());
        assert_eq!(
            fs::read(target.join("gentd.exe")).unwrap(),
            b"outside-canary"
        );
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        fs::remove_file(&link).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }
}
