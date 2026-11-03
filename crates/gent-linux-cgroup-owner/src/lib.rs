//! Real cgroup2 writes are restricted to a new owned child of an existing delegation.
//! This module never enables controllers, creates delegation, or issues Gent evidence.
#![cfg(target_os = "linux")]
use gent_types::SandboxResourceLimits;
use rustix::fs::{AtFlags, CWD, Mode, OFlags, ResolveFlags, fstatfs, mkdirat, openat2, unlinkat};
use std::{
    fs::File,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    process::Child,
    time::{Duration, Instant},
};
const CGROUP2_MAGIC: i64 = 0x6367_7270;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    PrivilegedCaller,
    InvalidPath,
    WrongFilesystem,
    NotDelegated,
    MissingControllers,
    MissingNamespaceDelegation,
    InvalidLimits,
    InvalidName,
    ExistingScope,
    Io,
    MalformedState,
    HelperIdentity,
    ScopeIdentity,
    CleanupIncomplete,
}
fn fd_at(parent: &File, name: &str, flags: OFlags) -> Result<File, Error> {
    openat2(
        parent,
        name,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
    )
    .map(File::from)
    .map_err(|_| Error::Io)
}
fn read(parent: &File, name: &str) -> Result<String, Error> {
    let mut text = String::new();
    fd_at(parent, name, OFlags::RDONLY)?
        .take(8193)
        .read_to_string(&mut text)
        .map_err(|_| Error::Io)?;
    if text.len() > 8192 || !text.is_ascii() {
        return Err(Error::MalformedState);
    }
    Ok(text)
}
fn write(parent: &File, name: &str, value: &str) -> Result<(), Error> {
    if value.len() > 128 || !value.is_ascii() {
        return Err(Error::MalformedState);
    }
    let n = fd_at(parent, name, OFlags::WRONLY)?
        .write(value.as_bytes())
        .map_err(|_| Error::Io)?;
    if n != value.len() {
        return Err(Error::Io);
    }
    Ok(())
}
fn field(text: &str, key: &str) -> Result<u64, Error> {
    let mut found = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next().ok_or(Error::MalformedState)?;
        let value = parts.next().ok_or(Error::MalformedState)?;
        if parts.next().is_some() {
            return Err(Error::MalformedState);
        }
        if name == key {
            if found.is_some() {
                return Err(Error::MalformedState);
            }
            found = Some(value.parse().map_err(|_| Error::MalformedState)?);
        }
    }
    found.ok_or(Error::MalformedState)
}
fn nonce_ok(nonce: &str) -> bool {
    nonce.len() == 32
        && nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn limits_budget(limits: SandboxResourceLimits) -> Result<u64, Error> {
    if limits.max_processes == 0
        || limits.max_memory_bytes == 0
        || !limits.max_memory_bytes.is_multiple_of(4096)
        || limits.max_cpu_time_ms == 0
    {
        return Err(Error::InvalidLimits);
    }
    limits
        .max_cpu_time_ms
        .checked_mul(1000)
        .ok_or(Error::InvalidLimits)
}
fn uid() -> Result<u32, Error> {
    let uid = rustix::process::geteuid().as_raw();
    if uid == 0 || uid != rustix::process::getuid().as_raw() {
        return Err(Error::PrivilegedCaller);
    }
    let status = std::fs::read_to_string("/proc/self/status").map_err(|_| Error::Io)?;
    let ids = status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:\t"))
        .ok_or(Error::MalformedState)?;
    let ids = ids
        .split_whitespace()
        .map(|n| n.parse::<u32>().map_err(|_| Error::MalformedState))
        .collect::<Result<Vec<_>, _>>()?;
    if ids.len() != 4 || ids.iter().any(|id| *id != uid) {
        return Err(Error::PrivilegedCaller);
    }
    for key in ["CapEff:\t", "CapPrm:\t", "CapAmb:\t"] {
        let caps = status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .ok_or(Error::MalformedState)?;
        if u64::from_str_radix(caps.trim(), 16).map_err(|_| Error::MalformedState)? != 0 {
            return Err(Error::PrivilegedCaller);
        }
    }
    Ok(uid)
}
// Bind the mount requirement to the opened directory's actual mount ID, not
// a guessed path or another cgroup2 mount. This is read-only admission.
fn namespace_delegated_mount(fdinfo: &str, mountinfo: &str) -> Result<(), Error> {
    if fdinfo.len() > 8192 || mountinfo.len() > 1024 * 1024 {
        return Err(Error::MalformedState);
    }
    let ids = fdinfo
        .lines()
        .filter_map(|l| l.strip_prefix("mnt_id:"))
        .collect::<Vec<_>>();
    if ids.len() != 1 {
        return Err(Error::MalformedState);
    }
    let id = ids[0]
        .trim()
        .parse::<u64>()
        .map_err(|_| Error::MalformedState)?;
    let mut matched = false;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            return Err(Error::MalformedState);
        };
        let fields = left.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 {
            return Err(Error::MalformedState);
        }
        if fields[0]
            .parse::<u64>()
            .map_err(|_| Error::MalformedState)?
            != id
        {
            continue;
        }
        if matched {
            return Err(Error::MalformedState);
        }
        matched = true;
        let after = right.split_whitespace().collect::<Vec<_>>();
        if after.len() != 3 || after[0] != "cgroup2" {
            return Err(Error::WrongFilesystem);
        }
        if !after[2].split(',').any(|o| o == "nsdelegate") {
            return Err(Error::MissingNamespaceDelegation);
        }
    }
    if !matched {
        return Err(Error::MalformedState);
    }
    Ok(())
}
fn require_namespace_delegation(fd: &File) -> Result<(), Error> {
    fn bounded(path: &str, limit: u64) -> Result<String, Error> {
        let mut text = String::new();
        File::open(path)
            .map_err(|_| Error::Io)?
            .take(limit + 1)
            .read_to_string(&mut text)
            .map_err(|_| Error::Io)?;
        if text.len() as u64 > limit {
            return Err(Error::MalformedState);
        }
        Ok(text)
    }
    namespace_delegated_mount(
        &bounded(&format!("/proc/self/fdinfo/{}", fd.as_raw_fd()), 8192)?,
        &bounded("/proc/self/mountinfo", 1024 * 1024)?,
    )
}
#[derive(Debug)]
pub struct DelegatedParent {
    fd: File,
    path: PathBuf,
    uid: u32,
}
impl DelegatedParent {
    /// Read-only admission. A root-owned or ordinary-directory fixture is never delegation.
    pub fn inspect(path: &Path) -> Result<Self, Error> {
        Self::inspect_inner(path, true)
    }
    fn inspect_inner(path: &Path, require_enabled: bool) -> Result<Self, Error> {
        let uid = uid()?;
        if !path.is_absolute()
            || !path.starts_with("/sys/fs/cgroup")
            || path.components().any(|p| {
                matches!(
                    p,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Err(Error::InvalidPath);
        }
        let fd = File::from(
            openat2(
                CWD,
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::NO_SYMLINKS,
            )
            .map_err(|_| Error::InvalidPath)?,
        );
        if fstatfs(&fd).map_err(|_| Error::Io)?.f_type as i64 != CGROUP2_MAGIC {
            return Err(Error::WrongFilesystem);
        }
        require_namespace_delegation(&fd)?;
        let m = fd.metadata().map_err(|_| Error::Io)?;
        if m.uid() != uid || m.mode() & 0o300 != 0o300 || m.mode() & 0o022 != 0 {
            return Err(Error::NotDelegated);
        }
        for name in ["cgroup.procs", "cgroup.subtree_control"] {
            let m = fd_at(&fd, name, OFlags::RDONLY)?
                .metadata()
                .map_err(|_| Error::Io)?;
            if m.uid() != uid {
                return Err(Error::NotDelegated);
            }
        }
        if read(&fd, "cgroup.type")?.trim() != "domain"
            || !read(&fd, "cgroup.procs")?.trim().is_empty()
        {
            return Err(Error::NotDelegated);
        }
        let enabled = read(&fd, "cgroup.subtree_control")?;
        if require_enabled
            && ["memory", "pids"]
                .iter()
                .any(|name| !enabled.split_whitespace().any(|c| c == *name))
        {
            return Err(Error::MissingControllers);
        }
        // Open only; no writes to parent or its ancestors.
        drop(fd_at(&fd, "cgroup.procs", OFlags::WRONLY).map_err(|_| Error::NotDelegated)?);
        Ok(Self {
            fd,
            path: path.to_owned(),
            uid,
        })
    }
}
/// Explicit service startup only. Enables controllers in the caller's OWN already
/// delegated service root; never creates delegation, moves a process or writes ancestors.
pub fn prepare_current_service() -> Result<(), Error> {
    let membership = std::fs::read_to_string("/proc/self/cgroup").map_err(|_| Error::Io)?;
    let relative = service_membership(&membership)?;
    let current = Path::new("/sys/fs/cgroup").join(relative);
    let parent = current.parent().ok_or(Error::InvalidPath)?;
    let own = DelegatedParent::inspect_inner(parent, false)?;
    let identity = own.fd.metadata().map_err(|_| Error::Io)?;
    let controller = fd_at(&own.fd, "controller", OFlags::RDONLY | OFlags::DIRECTORY)?;
    if controller.metadata().map_err(|_| Error::Io)?.uid() != own.uid {
        return Err(Error::NotDelegated);
    }
    let check = || -> Result<(), Error> {
        if std::fs::read_to_string("/proc/self/cgroup").map_err(|_| Error::Io)? != membership
            || !read(&own.fd, "cgroup.procs")?.trim().is_empty()
            || read(&controller, "cgroup.procs")?.trim() != std::process::id().to_string()
        {
            return Err(Error::NotDelegated);
        }
        let reopened = DelegatedParent::inspect_inner(parent, false)?;
        let m = reopened.fd.metadata().map_err(|_| Error::Io)?;
        if (m.dev(), m.ino()) != (identity.dev(), identity.ino()) {
            return Err(Error::ScopeIdentity);
        }
        // No sibling workload may acquire changed controller semantics.
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", own.fd.as_raw_fd()))
            .map_err(|_| Error::Io)?
        {
            let entry = entry.map_err(|_| Error::Io)?;
            if !entry.file_type().map_err(|_| Error::Io)?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if name == "controller" {
                continue;
            }
            if name != ".control" {
                return Err(Error::NotDelegated);
            }
            let control = fd_at(&own.fd, ".control", OFlags::RDONLY | OFlags::DIRECTORY)?;
            if !read(&control, "cgroup.procs")?.trim().is_empty()
                || field(&read(&control, "cgroup.events")?, "populated")? != 0
            {
                return Err(Error::NotDelegated);
            }
        }
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", controller.as_raw_fd()))
            .map_err(|_| Error::Io)?
        {
            if entry
                .map_err(|_| Error::Io)?
                .file_type()
                .map_err(|_| Error::Io)?
                .is_dir()
            {
                return Err(Error::NotDelegated);
            }
        }
        Ok(())
    };
    check()?;
    let available = read(&own.fd, "cgroup.controllers")?;
    if ["memory", "pids"]
        .iter()
        .any(|c| !available.split_whitespace().any(|v| v == *c))
    {
        return Err(Error::MissingControllers);
    }
    let enabled = read(&own.fd, "cgroup.subtree_control")?;
    let changes = ["memory", "pids"]
        .iter()
        .filter(|c| !enabled.split_whitespace().any(|v| v == **c))
        .map(|c| format!("+{c}"))
        .collect::<Vec<_>>()
        .join(" ");
    if !changes.is_empty() {
        write(&own.fd, "cgroup.subtree_control", &changes)?;
    }
    check()?;
    DelegatedParent::inspect(parent)?;
    Ok(())
}
fn service_membership(text: &str) -> Result<&str, Error> {
    let mut lines = text.lines();
    let relative = lines
        .next()
        .and_then(|l| l.strip_prefix("0::/"))
        .ok_or(Error::InvalidPath)?;
    let path = Path::new(relative);
    if lines.next().is_some()
        || path.is_absolute()
        || path
            .components()
            .any(|p| !matches!(p, std::path::Component::Normal(_)))
        || path.file_name().and_then(|v| v.to_str()) != Some("controller")
        || !path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.ends_with(".service"))
    {
        return Err(Error::InvalidPath);
    }
    Ok(relative)
}
#[cfg(test)]
mod service_tests {
    #[test]
    fn startup_only_selects_kernel_service_controller() {
        assert!(super::service_membership("0::/user.slice/gent.service/controller\n").is_ok());
        for text in [
            "0::/user.slice/controller\n",
            "0::/gent.service/work\n",
            "0::/gent.service/../controller\n",
            "0::/gent.service/controller\n0::/other\n",
            "0:://gent.service/controller\n",
        ] {
            assert!(super::service_membership(text).is_err(), "{text}");
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CpuSample {
    pub usage_usec: u64,
    pub budget_usec: u64,
    pub exceeded: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct Cleanup {
    pub empty: bool,
    pub removed: bool,
}
#[derive(Debug)]
pub struct CgroupScope {
    parent: DelegatedParent,
    fd: File,
    name: String,
    inode: u64,
    device: u64,
    budget_usec: u64,
    removed: bool,
}
impl CgroupScope {
    pub fn membership_path(&self) -> Result<String, Error> {
        let path = self.parent.path.join(&self.name);
        Ok(format!(
            "/{}",
            path.strip_prefix("/sys/fs/cgroup")
                .map_err(|_| Error::ScopeIdentity)?
                .to_string_lossy()
        ))
    }
    pub fn memory_limit_bytes(&self) -> Result<u64, Error> {
        read(&self.fd, "memory.max")?
            .trim()
            .parse()
            .map_err(|_| Error::MalformedState)
    }
    pub fn task_limit(&self) -> Result<u16, Error> {
        read(&self.fd, "pids.max")?
            .trim()
            .parse()
            .map_err(|_| Error::MalformedState)
    }
    pub fn peak_tasks(&self) -> Result<u64, Error> {
        read(&self.fd, "pids.peak")?
            .trim()
            .parse()
            .map_err(|_| Error::MalformedState)
    }
    pub fn monitored_cpu_budget_usec(&self) -> u64 {
        self.budget_usec
    }
    pub fn create(
        parent: DelegatedParent,
        nonce: &str,
        limits: SandboxResourceLimits,
    ) -> Result<Self, Error> {
        if !nonce_ok(nonce) {
            return Err(Error::InvalidName);
        }
        let budget_usec = limits_budget(limits)?;
        let name = format!("gent-{nonce}");
        mkdirat(&parent.fd, &name, Mode::from_bits_truncate(0o700)).map_err(|e| {
            if e == rustix::io::Errno::EXIST {
                Error::ExistingScope
            } else {
                Error::Io
            }
        })?;
        let fd = match fd_at(&parent.fd, &name, OFlags::RDONLY | OFlags::DIRECTORY) {
            Ok(fd) => fd,
            Err(e) => {
                let _ = unlinkat(&parent.fd, &name, AtFlags::REMOVEDIR);
                return Err(e);
            }
        };
        let Ok(m) = fd.metadata() else {
            let _ = unlinkat(&parent.fd, &name, AtFlags::REMOVEDIR);
            return Err(Error::Io);
        };
        let scope = Self {
            parent,
            fd,
            name,
            inode: m.ino(),
            device: m.dev(),
            budget_usec,
            removed: false,
        };
        if m.uid() != scope.parent.uid
            || fstatfs(&scope.fd).map_err(|_| Error::Io)?.f_type as i64 != CGROUP2_MAGIC
        {
            return Err(Error::ScopeIdentity);
        }
        for (key, value) in [
            ("memory.max", limits.max_memory_bytes.to_string()),
            ("memory.swap.max", "0".into()),
            ("memory.oom.group", "1".into()),
            ("pids.max", limits.max_processes.to_string()),
        ] {
            write(&scope.fd, key, &value)?;
            if read(&scope.fd, key)?.trim() != value {
                return Err(Error::InvalidLimits);
            }
        }
        // cpu.stat accounts the whole subtree even without the CPU controller.
        // This is monitored cumulative CPU, never a cpu.max rate/hard-total claim.
        let _ = field(&read(&scope.fd, "cpu.stat")?, "usage_usec")?;
        // Require tree kill and event support before any helper may enter.
        drop(fd_at(&scope.fd, "cgroup.kill", OFlags::WRONLY)?);
        if field(&read(&scope.fd, "cgroup.events")?, "populated")? != 0 {
            return Err(Error::ScopeIdentity);
        }
        Ok(scope)
    }
    /// Caller must start a pinned trusted helper blocked on its control pipe.
    /// This does not establish image trust or authorize an already-runnable workload.
    pub fn attach_blocked_helper(
        &self,
        child: &mut Child,
        expected_birth: &str,
    ) -> Result<(), Error> {
        if child.try_wait().map_err(|_| Error::Io)?.is_some() {
            return Err(Error::HelperIdentity);
        }
        let pid = child.id();
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|_| Error::HelperIdentity)?;
        let tail = stat
            .rsplit_once(')')
            .ok_or(Error::HelperIdentity)?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        if tail.get(19).copied() != Some(expected_birth)
            || tail.get(1).and_then(|p| p.parse::<u32>().ok()) != Some(std::process::id())
        {
            return Err(Error::HelperIdentity);
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .map_err(|_| Error::HelperIdentity)?;
        let ids = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:\t"))
            .ok_or(Error::HelperIdentity)?;
        if ids
            .split_whitespace()
            .any(|id| id.parse::<u32>().ok() != Some(self.parent.uid))
        {
            return Err(Error::HelperIdentity);
        }
        write(&self.fd, "cgroup.procs", &pid.to_string())?;
        let expected = self
            .parent
            .path
            .join(&self.name)
            .strip_prefix("/sys/fs/cgroup")
            .map_err(|_| Error::ScopeIdentity)?
            .to_string_lossy()
            .into_owned();
        if std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
            .map_err(|_| Error::HelperIdentity)?
            .trim()
            != format!("0::/{expected}")
        {
            let _ = write(&self.fd, "cgroup.kill", "1");
            return Err(Error::ScopeIdentity);
        }
        Ok(())
    }
    /// CPU accounting is monitored, NOT a hard total-CPU ceiling. No Gent attestation.
    pub fn sample_cpu_budget(&self) -> Result<CpuSample, Error> {
        let usage_usec = field(&read(&self.fd, "cpu.stat")?, "usage_usec")?;
        let exceeded = usage_usec >= self.budget_usec;
        if exceeded {
            write(&self.fd, "cgroup.kill", "1")?;
        }
        Ok(CpuSample {
            usage_usec,
            budget_usec: self.budget_usec,
            exceeded,
        })
    }
    pub fn terminate_and_remove(&mut self, deadline: Duration) -> Result<Cleanup, Error> {
        if self.removed {
            return Ok(Cleanup {
                empty: true,
                removed: true,
            });
        }
        if field(&read(&self.fd, "cgroup.events")?, "populated")? != 0 {
            write(&self.fd, "cgroup.kill", "1")?;
        }
        let start = Instant::now();
        while field(&read(&self.fd, "cgroup.events")?, "populated")? != 0 {
            if start.elapsed() >= deadline {
                return Err(Error::CleanupIncomplete);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let same = fd_at(
            &self.parent.fd,
            &self.name,
            OFlags::RDONLY | OFlags::DIRECTORY,
        )?
        .metadata()
        .map_err(|_| Error::Io)?;
        if same.ino() != self.inode || same.dev() != self.device {
            return Err(Error::ScopeIdentity);
        }
        unlinkat(&self.parent.fd, &self.name, AtFlags::REMOVEDIR)
            .map_err(|_| Error::CleanupIncomplete)?;
        self.removed = true;
        Ok(Cleanup {
            empty: true,
            removed: true,
        })
    }
}
impl Drop for CgroupScope {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.terminate_and_remove(Duration::from_millis(500));
        }
    }
}
#[cfg(test)]
mod tests;
