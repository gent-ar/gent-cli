//! Immutable executable snapshots. Capture binds an already-authorized origin;
//! it does not authorize a provider, change a run lock or create sandbox evidence.
use gent_types::RunVersionLock;
use rustix::fs::{MemfdFlags, SealFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    time::UNIX_EPOCH,
};

pub const REQUIRED_SEALS: SealFlags = SealFlags::WRITE
    .union(SealFlags::GROW)
    .union(SealFlags::SHRINK)
    .union(SealFlags::EXEC)
    .union(SealFlags::SEAL);
const MAX_IMAGE: u64 = 512 * 1024 * 1024;

pub struct SealedExecutable {
    file: File,
    origin: RunVersionLock,
    origin_file: File,
    origin_stamp: OriginStamp,
    bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginStamp {
    device: u64,
    inode: u64,
    ctime: i64,
    ctime_nsec: i64,
    identity: String,
}
impl OriginStamp {
    fn capture(value: &Metadata) -> Result<Self, &'static str> {
        Ok(Self {
            device: value.dev(),
            inode: value.ino(),
            ctime: value.ctime(),
            ctime_nsec: value.ctime_nsec(),
            identity: identity(value)?,
        })
    }
    fn matches(&self, value: &Metadata) -> bool {
        Self::capture(value).as_ref() == Ok(self)
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedImageReady {
    pub schema: u8,
    pub descriptor: u32,
    pub origin: RunVersionLock,
    pub origin_stamp: OriginStamp,
    pub bytes: u64,
}
#[derive(Debug)]
pub struct RetainedImage {
    image: File,
    source: File,
    ready: SealedImageReady,
}
impl RetainedImage {
    pub fn inspect(
        pid: u32,
        ready: &SealedImageReady,
        expected: &RunVersionLock,
    ) -> Result<Self, &'static str> {
        if ready.schema != 1
            || ready.descriptor > 1023
            || ready.origin != *expected
            || ready.origin_stamp.identity != expected.file_identity
            || ready.bytes < 64
            || ready.bytes > MAX_IMAGE
        {
            return Err("sealed readiness binding");
        }
        let image = File::open(format!("/proc/{pid}/fd/{}", ready.descriptor))
            .map_err(|_| "sealed descriptor unavailable")?;
        if !rustix::fs::fcntl_get_seals(&image)
            .map_err(|_| "sealed descriptor type")?
            .contains(REQUIRED_SEALS)
            || image.metadata().map_err(|_| "sealed metadata")?.len() != ready.bytes
        {
            return Err("sealed kernel properties");
        }
        let mut reader = &image;
        let mut buffer = [0u8; 65536];
        let mut hash = Sha256::new();
        let mut count = 0u64;
        loop {
            let n = reader
                .read(&mut buffer)
                .map_err(|_| "sealed inspection read")?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > MAX_IMAGE {
                return Err("sealed inspection bound");
            }
            hash.update(&buffer[..n]);
        }
        if count != ready.bytes || format!("{:x}", hash.finalize()) != expected.digest_sha256 {
            return Err("sealed inspection digest");
        }
        let source = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&expected.canonical_path)
            .map_err(|_| "origin retention")?;
        let retained = Self {
            image,
            source,
            ready: ready.clone(),
        };
        retained.source_current()?;
        Ok(retained)
    }
    pub fn source_current(&self) -> Result<(), &'static str> {
        if !self.ready.origin_stamp.matches(
            &self
                .source
                .metadata()
                .map_err(|_| "origin retained metadata")?,
        ) || !self.ready.origin_stamp.matches(
            &std::fs::metadata(&self.ready.origin.canonical_path)
                .map_err(|_| "origin path unavailable")?,
        ) {
            return Err("origin generation changed");
        }
        nix::unistd::faccessat(
            &self.source,
            c"",
            nix::unistd::AccessFlags::X_OK,
            nix::fcntl::AtFlags::AT_EMPTY_PATH | nix::fcntl::AtFlags::AT_EACCESS,
        )
        .map_err(|_| "origin execution revoked")
    }
    pub fn matches_executable(&self, pid: u32) -> Result<bool, &'static str> {
        self.source_current()?;
        if !rustix::fs::fcntl_get_seals(&self.image)
            .map_err(|_| "live seals unavailable")?
            .contains(REQUIRED_SEALS)
        {
            return Err("live image seals changed");
        }
        let expected = self
            .image
            .metadata()
            .map_err(|_| "retained image unavailable")?;
        let current = std::fs::metadata(format!("/proc/{pid}/exe"))
            .map_err(|_| "live executable unavailable")?;
        Ok(expected.dev() == current.dev() && expected.ino() == current.ino())
    }
    pub fn public_proof(&self) -> Result<serde_json::Value, &'static str> {
        let metadata = self
            .image
            .metadata()
            .map_err(|_| "retained image unavailable")?;
        Ok(
            serde_json::json!({"kind":"sealedMemfd","sha256":self.ready.origin.digest_sha256,"sourceIdentity":self.ready.origin.file_identity,"device":metadata.dev(),"inode":metadata.ino(),"bytes":metadata.len(),"seals":rustix::fs::fcntl_get_seals(&self.image).map_err(|_|"image seals unavailable")?.bits()}),
        )
    }
}
impl std::fmt::Debug for SealedExecutable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedExecutable")
            .field("sha256", &self.origin.digest_sha256)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
fn identity(m: &Metadata) -> Result<String, &'static str> {
    Ok(format!(
        "{}:{}",
        m.len(),
        m.modified()
            .map_err(|_| "origin timestamp")?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |v| v.as_nanos())
    ))
}
fn unchanged(a: &Metadata, b: &Metadata) -> Result<bool, &'static str> {
    Ok(a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
        && identity(a)? == identity(b)?)
}
fn elf_native(header: &[u8]) -> bool {
    #[cfg(target_arch = "x86_64")]
    let machine = 62u16;
    #[cfg(target_arch = "aarch64")]
    let machine = 183u16;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return false;
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        header.len() >= 64
            && &header[..4] == b"\x7fELF"
            && header[4] == 2
            && header[5] == 1
            && header[6] == 1
            && u16::from_le_bytes([header[18], header[19]]) == machine
    }
}
impl SealedExecutable {
    pub fn capture(origin: &RunVersionLock) -> Result<Self, &'static str> {
        Self::capture_observed(origin, |_| {})
    }
    fn capture_observed(
        origin: &RunVersionLock,
        mut observed: impl FnMut(u64),
    ) -> Result<Self, &'static str> {
        let path = Path::new(&origin.canonical_path);
        if !path.is_absolute() || path.canonicalize().map_err(|_| "origin path")? != path {
            return Err("origin redirected");
        }
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path)
            .map_err(|_| "origin open")?;
        let before = input.metadata().map_err(|_| "origin metadata")?;
        if !before.is_file()
            || before.len() < 64
            || before.len() > MAX_IMAGE
            || identity(&before)? != origin.file_identity
        {
            return Err("origin identity or size");
        }
        // Preserve source execute permissions/noexec policy, not just read permission.
        nix::unistd::faccessat(
            &input,
            c"",
            nix::unistd::AccessFlags::X_OK,
            nix::fcntl::AtFlags::AT_EMPTY_PATH | nix::fcntl::AtFlags::AT_EACCESS,
        )
        .map_err(|_| "origin execution denied")?;
        // Explicit executable memfd. Never retry without EXEC or change vm.memfd_noexec.
        let mut file = File::from(
            rustix::fs::memfd_create(
                c"gent-provider",
                MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING | MemfdFlags::EXEC,
            )
            .map_err(|_| "executable memfd unavailable or denied")?,
        );
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut count = 0u64;
        let mut first = true;
        loop {
            let n = input.read(&mut buffer).map_err(|_| "origin read")?;
            if n == 0 {
                break;
            }
            if first {
                if !elf_native(&buffer[..n]) {
                    return Err("native ELF required");
                }
                first = false;
            }
            count = count
                .checked_add(n as u64)
                .filter(|v| *v <= MAX_IMAGE)
                .ok_or("origin grew")?;
            file.write_all(&buffer[..n]).map_err(|_| "snapshot write")?;
            hash.update(&buffer[..n]);
            observed(count);
        }
        if count != before.len()
            || format!("{:x}", hash.finalize()) != origin.digest_sha256
            || !unchanged(
                &before,
                &input.metadata().map_err(|_| "origin final handle")?,
            )?
            || !unchanged(
                &before,
                &std::fs::metadata(path).map_err(|_| "origin deleted")?,
            )?
        {
            return Err("origin changed during snapshot");
        }
        // EXEC also freezes executable permission bits. Contents, size and future seals freeze too.
        rustix::fs::fcntl_add_seals(&file, REQUIRED_SEALS).map_err(|_| "snapshot sealing")?;
        if !rustix::fs::fcntl_get_seals(&file)
            .map_err(|_| "snapshot seals unavailable")?
            .contains(REQUIRED_SEALS)
        {
            return Err("snapshot not immutable");
        }
        let readonly = File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))
            .map_err(|_| "snapshot readonly reopen")?;
        let first = file.metadata().map_err(|_| "snapshot metadata")?;
        let second = readonly
            .metadata()
            .map_err(|_| "snapshot readonly metadata")?;
        if first.dev() != second.dev() || first.ino() != second.ino() {
            return Err("snapshot readonly identity changed");
        }
        drop(file);
        Ok(Self {
            file: readonly,
            origin: origin.clone(),
            origin_file: input,
            origin_stamp: OriginStamp::capture(&before)?,
            bytes: count,
        })
    }
    pub fn ready(&self) -> Result<SealedImageReady, &'static str> {
        Ok(SealedImageReady {
            schema: 1,
            descriptor: self
                .file
                .as_raw_fd()
                .try_into()
                .map_err(|_| "snapshot fd range")?,
            origin: self.origin.clone(),
            origin_stamp: self.origin_stamp.clone(),
            bytes: self.bytes,
        })
    }
    pub fn source_current(&self) -> Result<(), &'static str> {
        if !self.origin_stamp.matches(
            &self
                .origin_file
                .metadata()
                .map_err(|_| "origin retained metadata")?,
        ) || !self.origin_stamp.matches(
            &std::fs::metadata(&self.origin.canonical_path)
                .map_err(|_| "origin path disappeared")?,
        ) {
            return Err("origin changed before execution");
        }
        Ok(())
    }
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
    pub fn origin(&self) -> &RunVersionLock {
        &self.origin
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    /// Trusted HOST probe only, not containment evidence. The caller retains
    /// this snapshot until the child settles. The kernel opens our exact RO
    /// sealed FD through our live PID; no mutable source pathname executes.
    pub(crate) fn host_command(&self) -> Result<std::process::Command, &'static str> {
        use std::os::unix::process::CommandExt;
        self.source_current()?;
        let status = std::fs::read_to_string("/proc/self/status")
            .map_err(|_| "host exec privilege status")?;
        if rustix::process::geteuid().as_raw() == 0
            || rustix::process::getuid() != rustix::process::geteuid()
            || !status.lines().any(|l| l == "NoNewPrivs:\t1")
            || ["CapEff:\t", "CapPrm:\t", "CapAmb:\t"].iter().any(|key| {
                status
                    .lines()
                    .find_map(|l| l.strip_prefix(key))
                    .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
                    != Some(0)
            })
        {
            return Err("host exec requires unprivileged identity");
        }
        if !rustix::fs::fcntl_get_seals(&self.file)
            .map_err(|_| "host exec seals")?
            .contains(REQUIRED_SEALS)
        {
            return Err("host exec seals changed");
        }
        let mut command = std::process::Command::new(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.file.as_raw_fd()
        ));
        command.arg0(&self.origin.canonical_path);
        Ok(command)
    }
    pub(crate) fn host_executable_matches(&self, pid: u32) -> Result<bool, &'static str> {
        self.source_current()?;
        let expected = self.file.metadata().map_err(|_| "host sealed metadata")?;
        let observed = std::fs::metadata(format!("/proc/{pid}/exe"))
            .map_err(|_| "host executable unavailable")?;
        Ok(expected.dev() == observed.dev() && expected.ino() == observed.ino())
    }
    pub fn exec(
        &self,
        args: &[String],
        environment: &BTreeMap<String, String>,
    ) -> Result<std::convert::Infallible, &'static str> {
        if args.len() > 128
            || environment.len() > 64
            || args.iter().any(|v| v.len() > 16384)
            || environment
                .iter()
                .any(|(k, v)| k.contains('=') || k.is_empty() || v.len() > 16384)
        {
            return Err("sealed exec argument bounds");
        }
        let status =
            std::fs::read_to_string("/proc/self/status").map_err(|_| "exec privilege status")?;
        if !status.lines().any(|v| v == "NoNewPrivs:\t1")
            || ["CapEff:\t", "CapPrm:\t", "CapAmb:\t"].iter().any(|key| {
                status
                    .lines()
                    .find_map(|v| v.strip_prefix(key))
                    .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
                    != Some(0)
            })
        {
            return Err("sealed exec requires no-new-privileges and no capabilities");
        }
        if !rustix::fs::fcntl_get_seals(&self.file)
            .map_err(|_| "exec seals")?
            .contains(REQUIRED_SEALS)
        {
            return Err("exec seals changed");
        }
        let argv = std::iter::once(self.origin.canonical_path.as_str())
            .chain(args.iter().map(String::as_str))
            .map(CString::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "exec argument NUL")?;
        let env = environment
            .iter()
            .map(|(k, v)| CString::new(format!("{k}={v}")))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "exec environment NUL")?;
        nix::unistd::execveat(
            &self.file,
            c"",
            &argv,
            &env,
            nix::fcntl::AtFlags::AT_EMPTY_PATH,
        )
        .map_err(|error| match error {
            nix::errno::Errno::ETXTBSY => "sealed exec busy",
            nix::errno::Errno::EACCES | nix::errno::Errno::EPERM => "sealed exec permission denied",
            nix::errno::Errno::ENOENT => "sealed exec loader absent",
            nix::errno::Errno::ENOEXEC => "sealed exec invalid image",
            _ => "sealed exec denied",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn origin() -> (tempfile::TempDir, RunVersionLock) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image");
        std::fs::copy("/usr/bin/true", &path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let lock = gent_drivers::lock::capture("qa", &path, "public system ELF", "QA").unwrap();
        (dir, lock)
    }
    #[test]
    fn retained_origin_rejects_permission_change_and_identical_replacement() {
        for replace in [false, true] {
            let (dir, lock) = origin();
            let snapshot = SealedExecutable::capture(&lock).unwrap();
            let retained =
                RetainedImage::inspect(std::process::id(), &snapshot.ready().unwrap(), &lock)
                    .unwrap();
            assert!(retained.source_current().is_ok());
            if replace {
                let old = dir.path().join("old");
                std::fs::rename(&lock.canonical_path, &old).unwrap();
                std::fs::copy(old, &lock.canonical_path).unwrap();
            } else {
                std::fs::set_permissions(
                    &lock.canonical_path,
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            assert!(retained.source_current().is_err());
        }
    }
    #[test]
    fn descriptor_with_other_sealed_bytes_cannot_substitute_for_origin() {
        let (dir, lock) = origin();
        let original = SealedExecutable::capture(&lock).unwrap();
        let other_path = dir.path().join("other");
        std::fs::copy("/usr/bin/false", &other_path).unwrap();
        let other_lock = gent_drivers::lock::capture("qa", &other_path, "qa", "qa").unwrap();
        let other = SealedExecutable::capture(&other_lock).unwrap();
        let mut forged = original.ready().unwrap();
        forged.descriptor = other.as_fd().as_raw_fd() as u32;
        assert!(RetainedImage::inspect(std::process::id(), &forged, &lock).is_err());
    }
    #[test]
    fn sealed_snapshot_rejects_write_grow_shrink_and_survives_origin_delete() {
        let (_dir, lock) = origin();
        let snapshot = SealedExecutable::capture(&lock).unwrap();
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/proc/self/fd/{}", snapshot.as_fd().as_raw_fd()))
            .unwrap();
        assert_eq!(
            rustix::io::pwrite(&writer, b"x", 0),
            Err(rustix::io::Errno::PERM)
        );
        assert!(writer.set_len(snapshot.bytes + 1).is_err());
        assert!(writer.set_len(snapshot.bytes - 1).is_err());
        drop(writer);
        std::fs::remove_file(&lock.canonical_path).unwrap();
        assert!(
            rustix::fs::fcntl_get_seals(snapshot.as_fd())
                .unwrap()
                .contains(REQUIRED_SEALS)
        );
    }
    #[test]
    fn concurrent_inplace_write_with_restored_mtime_is_rejected() {
        let (_dir, lock) = origin();
        let modified = std::fs::metadata(&lock.canonical_path)
            .unwrap()
            .modified()
            .unwrap();
        let mut done = false;
        assert!(
            SealedExecutable::capture_observed(&lock, |_| {
                if !done {
                    done = true;
                    let mut writer = OpenOptions::new()
                        .write(true)
                        .open(&lock.canonical_path)
                        .unwrap();
                    writer.write_all(b"changed").unwrap();
                    writer
                        .set_times(std::fs::FileTimes::new().set_modified(modified))
                        .unwrap();
                }
            })
            .is_err()
        );
    }
    #[test]
    fn concurrent_delete_and_equal_content_replacement_are_rejected() {
        for replace in [false, true] {
            let (dir, lock) = origin();
            let mut done = false;
            assert!(
                SealedExecutable::capture_observed(&lock, |_| {
                    if !done {
                        done = true;
                        let old = dir.path().join("old");
                        std::fs::rename(&lock.canonical_path, &old).unwrap();
                        if replace {
                            std::fs::copy(old, &lock.canonical_path).unwrap();
                        }
                    }
                })
                .is_err()
            );
        }
    }
    #[test]
    fn non_executable_and_script_origins_are_rejected() {
        let (_dir, lock) = origin();
        std::fs::set_permissions(&lock.canonical_path, std::fs::Permissions::from_mode(0o600))
            .unwrap();
        // Root's access semantics still deny X_OK when all execute bits are absent.
        assert!(SealedExecutable::capture(&lock).is_err());
        std::fs::write(&lock.canonical_path, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&lock.canonical_path, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let lock =
            gent_drivers::lock::capture("qa", Path::new(&lock.canonical_path), "qa", "qa").unwrap();
        assert!(SealedExecutable::capture(&lock).is_err());
    }
}
