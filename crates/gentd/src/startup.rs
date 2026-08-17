//! Platform-neutral startup values kept outside the composition root.

use std::path::PathBuf;

pub(super) fn default_data_dir() -> PathBuf {
    directories::BaseDirs::new().map_or_else(
        || PathBuf::from(".gentd"),
        |directories| directories.home_dir().join(".gentd"),
    )
}

pub(super) fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
