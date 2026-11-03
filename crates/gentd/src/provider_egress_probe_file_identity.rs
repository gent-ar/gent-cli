//! Safe OS metadata only: no unstable std API, unsafe Rust or file-data reads.
use super::{EgressError, File, Instant, MAX_NODES, OsString, Path};
#[derive(Clone, Copy)]
pub(super) struct Snapshot {
    pub volume: u64,
    pub index: u64,
    pub directory: bool,
    pub bytes: u64,
    pub links: u64,
}
impl Snapshot {
    pub fn capture(file: &File) -> Result<Self, EgressError> {
        let m = file.metadata().map_err(|_| EgressError::Authentication)?;
        if !m.is_file() && !m.is_dir() {
            return Err(EgressError::Authentication);
        }
        #[cfg(windows)]
        {
            let info =
                winapi_util::file::information(file).map_err(|_| EgressError::Authentication)?;
            if info.file_attributes() & 0x400 != 0
                || !winapi_util::file::typ(file)
                    .map_err(|_| EgressError::Authentication)?
                    .is_disk()
            {
                return Err(EgressError::Authentication);
            }
            Ok(Self {
                volume: info.volume_serial_number(),
                index: info.file_index(),
                directory: m.is_dir(),
                bytes: info.file_size(),
                links: info.number_of_links(),
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                volume: m.dev(),
                index: m.ino(),
                directory: m.is_dir(),
                bytes: m.len(),
                links: m.nlink(),
            })
        }
        #[cfg(not(any(windows, unix)))]
        Err(EgressError::Authentication)
    }
    pub fn same(&self, other: &Self) -> bool {
        self.volume == other.volume
            && self.index == other.index
            && self.directory == other.directory
            && (self.directory
                || (self.bytes == other.bytes && self.links == 1 && other.links == 1))
    }
}
pub(super) fn open_path(path: &Path) -> Result<File, EgressError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| EgressError::Authentication)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(EgressError::Authentication);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x81)
            .share_mode(1)
            .custom_flags(0x0200_0000 | 0x0020_0000);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::DIRECTORY)
                .bits()
                .cast_signed(),
        );
    }
    let file = options
        .open(path)
        .map_err(|_| EgressError::Authentication)?;
    Snapshot::capture(&file)?;
    Ok(file)
}
pub(super) fn open_child(parent: &File, path: &Path) -> Result<File, EgressError> {
    #[cfg(windows)]
    {
        use cap_primitives::fs::{FollowSymlinks, OpenOptions, OpenOptionsExt};
        let name = path.file_name().ok_or(EgressError::Authentication)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .access_mode(0x10081)
            .share_mode(1)
            .custom_flags(0x0200_0000 | 0x0020_0000);
        options
            ._cap_fs_ext_follow(FollowSymlinks::No)
            ._cap_fs_ext_maybe_dir(true);
        let file = cap_primitives::fs::open(parent, Path::new(name), &options)
            .map_err(|_| EgressError::Authentication)?;
        Snapshot::capture(&file)?;
        Ok(file)
    }
    #[cfg(target_os = "linux")]
    {
        let name = path.file_name().ok_or(EgressError::Authentication)?;
        let flags = rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_MAGICLINKS
            | rustix::fs::ResolveFlags::NO_XDEV;
        let fd = rustix::fs::openat2(
            parent,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
            flags,
        )
        .map_err(|_| EgressError::Authentication)?;
        let file = File::from(fd);
        Snapshot::capture(&file)?;
        Ok(file)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = parent;
        open_path(path)
    }
}
pub(super) fn names(
    file: &File,
    path: &Path,
    deadline: Instant,
) -> Result<Vec<OsString>, EgressError> {
    let mut names = Vec::new();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut dir = rustix::fs::Dir::read_from(file).map_err(|_| EgressError::Authentication)?;
        while let Some(entry) = dir.read() {
            let entry = entry.map_err(|_| EgressError::Authentication)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if Instant::now() >= deadline || names.len() >= MAX_NODES {
                return Err(EgressError::Authentication);
            }
            names.push(std::ffi::OsStr::from_bytes(name).to_os_string());
        }
        let _ = path;
    }
    #[cfg(windows)]
    {
        let _ = path;
        for entry in
            cap_primitives::fs::read_base_dir(file).map_err(|_| EgressError::Authentication)?
        {
            if Instant::now() >= deadline || names.len() >= MAX_NODES {
                return Err(EgressError::Authentication);
            }
            names.push(entry.map_err(|_| EgressError::Authentication)?.file_name());
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = file;
        for entry in std::fs::read_dir(path).map_err(|_| EgressError::Authentication)? {
            if Instant::now() >= deadline || names.len() >= MAX_NODES {
                return Err(EgressError::Authentication);
            }
            names.push(entry.map_err(|_| EgressError::Authentication)?.file_name());
        }
    }
    Ok(names)
}
