use std::path::{Path, PathBuf};

pub fn host_absolute_path(posix: &str) -> PathBuf {
    if cfg!(windows) {
        Path::new("C:\\").join(posix.trim_start_matches('/'))
    } else {
        PathBuf::from(posix)
    }
}
