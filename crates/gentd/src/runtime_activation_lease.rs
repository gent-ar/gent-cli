use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum LeaseFailure {
    #[error("runtime activation is blocked by an active owner or installer")]
    Busy,
    #[error("runtime activation lease is unavailable")]
    Unavailable,
    #[error("runtime owner epoch cannot be verified without changing its ledger")]
    LedgerUnavailable,
    #[error("runtime owner epoch changed before activation")]
    EpochMismatch,
    #[error("runtime owner did not durably quiesce before repair activation")]
    NotClosed,
    #[error("runtime activation requires plain absolute store and data paths")]
    InvalidPath,
}
pub(super) struct ActivationLease {
    _host: File,
    _store: File,
    pub(super) data_dir: PathBuf,
    pub(super) root: PathBuf,
    pub(super) origin_host_epoch: Option<u64>,
    pub(super) origin_ingress: Option<gent_ports::IngressMode>,
}
impl ActivationLease {
    pub(super) fn acquire(root: &Path, data: &Path, epoch: Option<u64>) -> Result<Self, String> {
        Self::open(root, data, epoch, true).map_err(|e| e.to_string())
    }
    pub(super) fn existing(root: &Path, data: &Path, epoch: Option<u64>) -> Result<Self, String> {
        Self::existing_checked(root, data, epoch).map_err(|e| e.to_string())
    }
    pub(super) fn existing_checked(
        root: &Path,
        data: &Path,
        epoch: Option<u64>,
    ) -> Result<Self, LeaseFailure> {
        Self::open(root, data, epoch, false)
    }
    fn open(
        root: &Path,
        data: &Path,
        epoch: Option<u64>,
        create: bool,
    ) -> Result<Self, LeaseFailure> {
        if !data.is_absolute()
            || !root.is_absolute()
            || data.parent().is_none()
            || root.parent().is_none()
        {
            return Err(LeaseFailure::InvalidPath);
        }
        let data_dir = directory(data, create)?;
        let host = acquire(&data_dir.join("gentd.lock"), create)?;
        let state = gent_store::SqliteLedger::read_host_state(data_dir.join("gent.db"))
            .map_err(|_| LeaseFailure::LedgerUnavailable)?;
        let origin_host_epoch = state.map(|s| s.epoch.0);
        if epoch.is_some_and(|e| Some(e) != origin_host_epoch) {
            return Err(LeaseFailure::EpochMismatch);
        }
        if epoch.is_some() && state.is_none_or(|s| s.mode != gent_ports::IngressMode::Closed) {
            return Err(LeaseFailure::NotClosed);
        }
        let root = directory(root, create)?;
        let store = acquire(&root.join(".install.lock"), create)?;
        Ok(Self {
            _host: host,
            _store: store,
            data_dir,
            root,
            origin_host_epoch,
            origin_ingress: state.map(|s| s.mode),
        })
    }
}
fn directory(path: &Path, create: bool) -> Result<PathBuf, LeaseFailure> {
    if create {
        std::fs::create_dir_all(path).map_err(|_| LeaseFailure::Unavailable)?;
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| LeaseFailure::Unavailable)?;
    if !metadata.is_dir() || super::files::redirected(&metadata) {
        return Err(LeaseFailure::InvalidPath);
    }
    path.canonicalize().map_err(|_| LeaseFailure::Unavailable)
}
fn acquire(path: &Path, create: bool) -> Result<File, LeaseFailure> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || super::files::redirected(&m)) {
        return Err(LeaseFailure::InvalidPath);
    }
    let file = OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| LeaseFailure::Unavailable)?;
    file.try_lock_exclusive().map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock
            || e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            LeaseFailure::Busy
        } else {
            LeaseFailure::Unavailable
        }
    })?;
    Ok(file)
}
