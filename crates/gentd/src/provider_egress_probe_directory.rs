//! Exact fresh probe tree. No cleanup of Unknown scopes or file contents read.
use super::EgressError;
use super::probe_process::ProbeFailure;
use std::time::{Duration, Instant};
use std::{
    ffi::OsString,
    fs::File,
    path::{Path, PathBuf},
};
#[path = "provider_egress_probe_file_identity.rs"]
mod identity;
#[cfg(windows)]
#[path = "provider_egress_probe_unlink.rs"]
mod unlink;
use identity::{Snapshot, open_child, open_path};
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 16;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub(super) struct Directory {
    root: PathBuf,
    known: bool,
    lock: Option<File>,
    parent: File,
    birth: Snapshot,
    pending: Option<Vec<Option<Entry>>>,
    failure: Option<ProbeFailure>,
}
struct Entry {
    path: PathBuf,
    #[cfg(target_os = "linux")]
    name: OsString,
    lock: File,
    #[cfg(target_os = "linux")]
    parent: File,
    snapshot: Snapshot,
}
impl Directory {
    pub fn create(parent: &Path) -> Result<Self, EgressError> {
        let parent_path = std::fs::canonicalize(parent).map_err(|_| EgressError::Authentication)?;
        let parent = open_path(&parent_path)?;
        let root = parent_path.join(format!(
            "credential-probe-{}",
            uuid::Uuid::new_v4().simple()
        ));
        #[cfg(windows)]
        cap_primitives::fs::create_dir(
            &parent,
            Path::new(root.file_name().unwrap()),
            &cap_primitives::fs::DirOptions::new(),
        )
        .map_err(|_| EgressError::Authentication)?;
        #[cfg(target_os = "linux")]
        rustix::fs::mkdirat(
            &parent,
            root.file_name().unwrap(),
            rustix::fs::Mode::from_bits_truncate(0o700),
        )
        .map_err(|_| EgressError::Authentication)?;
        #[cfg(not(any(windows, target_os = "linux")))]
        std::fs::create_dir(&root).map_err(|_| EgressError::Authentication)?;
        let lock = open_child(&parent, &root)?;
        let birth = Snapshot::capture(&lock)?;
        if !birth.directory {
            return Err(EgressError::Authentication);
        }
        Ok(Self {
            root,
            known: true,
            lock: Some(lock),
            parent,
            birth,
            pending: None,
            failure: None,
        })
    }
    pub fn path(&self) -> &Path {
        &self.root
    }
    pub fn in_flight(&mut self) {
        self.known = false;
    }
    pub fn settled(&mut self) {
        self.known = true;
    }
    pub fn take_cleanup_failure(&mut self) -> ProbeFailure {
        self.failure.take().unwrap_or_else(ProbeFailure::known)
    }
    pub fn cleanup(&mut self) -> Result<(), EgressError> {
        if !self.known || self.pending.is_some() {
            return Err(EgressError::Authentication);
        }
        if self.lock.is_none() {
            return Ok(());
        }
        if !self
            .birth
            .same(&Snapshot::capture(self.lock.as_ref().unwrap())?)
        {
            return Err(EgressError::Authentication);
        }
        let mut walk = Walk {
            volume: self.birth.volume,
            deadline: Instant::now() + Duration::from_secs(10),
            total: 0,
            entries: Vec::new(),
        };
        collect(
            &self.root,
            &self.parent,
            Some(
                self.lock
                    .as_ref()
                    .unwrap()
                    .try_clone()
                    .map_err(|_| EgressError::Authentication)?,
            ),
            0,
            &mut walk,
        )?;
        let deadline = walk.deadline;
        let mut entries = walk.entries.into_iter().map(Some).collect::<Vec<_>>();
        // Validate and pin the complete finite tree before any mutation.
        for index in (0..entries.len()).rev() {
            let entry = entries[index].as_ref().unwrap();
            let current = Snapshot::capture(&entry.lock)?;
            if Instant::now() >= deadline || !entry.snapshot.same(&current) {
                return Err(EgressError::Authentication);
            }
            #[cfg(windows)]
            if let Err(failure) = unlink::mark(&entry.lock, current, deadline) {
                self.failure = Some(failure);
                self.pending = Some(entries);
                return Err(EgressError::Authentication);
            }
            #[cfg(target_os = "linux")]
            {
                let check = open_child(&entry.parent, &entry.path)?;
                if !entry.snapshot.same(&Snapshot::capture(&check)?) {
                    return Err(EgressError::Authentication);
                }
                drop(check);
                let flags = if entry.snapshot.directory {
                    rustix::fs::AtFlags::REMOVEDIR
                } else {
                    rustix::fs::AtFlags::empty()
                };
                rustix::fs::unlinkat(&entry.parent, &entry.name, flags)
                    .map_err(|_| EgressError::Authentication)?;
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                if !entry.snapshot.directory {
                    return Err(EgressError::Authentication);
                }
                std::fs::remove_dir(&entry.path).map_err(|_| EgressError::Authentication)?;
            }
            if entry.path == self.root {
                drop(self.lock.take());
            }
            drop(entries[index].take()); // Complete only the acknowledged object's unlink.
        }
        Ok(())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if self.known {
            let _ = self.cleanup();
        }
    }
}
struct Walk {
    volume: u64,
    deadline: Instant,
    total: u64,
    entries: Vec<Entry>,
}
fn collect(
    path: &Path,
    parent: &File,
    existing: Option<File>,
    depth: usize,
    walk: &mut Walk,
) -> Result<(), EgressError> {
    if Instant::now() >= walk.deadline || depth > MAX_DEPTH || walk.entries.len() >= MAX_NODES {
        return Err(EgressError::Authentication);
    }
    let lock = match existing {
        Some(file) => file,
        None => open_child(parent, path)?,
    };
    let snapshot = Snapshot::capture(&lock)?;
    if snapshot.volume != walk.volume {
        return Err(EgressError::Authentication);
    }
    if !snapshot.directory {
        if snapshot.links != 1 || snapshot.bytes > MAX_FILE_BYTES {
            return Err(EgressError::Authentication);
        }
        walk.total = walk
            .total
            .checked_add(snapshot.bytes)
            .filter(|v| *v <= MAX_TOTAL_BYTES)
            .ok_or(EgressError::Authentication)?;
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(EgressError::Authentication);
    }
    #[cfg(windows)]
    if snapshot.links != 1 {
        return Err(EgressError::Authentication);
    }
    let children = if snapshot.directory {
        identity::names(&lock, path, walk.deadline)?
    } else {
        Vec::new()
    };
    let child_parent = lock.try_clone().map_err(|_| EgressError::Authentication)?;
    walk.entries.push(Entry {
        path: path.to_path_buf(),
        #[cfg(target_os = "linux")]
        name: path
            .file_name()
            .ok_or(EgressError::Authentication)?
            .to_os_string(),
        lock,
        #[cfg(target_os = "linux")]
        parent: parent
            .try_clone()
            .map_err(|_| EgressError::Authentication)?,
        snapshot,
    });
    for name in children {
        collect(&path.join(name), &child_parent, None, depth + 1, walk)?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "provider_egress_probe_directory_tests.rs"]
mod tests;
