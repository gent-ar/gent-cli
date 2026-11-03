//! Reviewed self-contained Node entries are data, never executable memfds.
use gent_types::RunVersionLock;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    time::UNIX_EPOCH,
};
fn identity(m: &Metadata) -> Result<String, &'static str> {
    Ok(format!(
        "{}:{}",
        m.len(),
        m.modified()
            .map_err(|_| "entry time")?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |v| v.as_nanos())
    ))
}
fn stamp(m: &Metadata) -> Result<(u64, u64, i64, i64, String), &'static str> {
    Ok((m.dev(), m.ino(), m.ctime(), m.ctime_nsec(), identity(m)?))
}
pub(crate) fn contract(args: &[String]) -> Option<(crate::node_profile::Role, RunVersionLock)> {
    if args.len() != 2 {
        return None;
    }
    let role = crate::node_profile::Role::from_server(&args[0])?;
    let lock: RunVersionLock = serde_json::from_str(args.get(1)?).ok()?;
    if lock.provider != "node-mcp-entry"
        || lock.compatibility_entry != format!("gent-node-bundle:{}:v1", args[0])
    {
        return None;
    }
    Some((role, lock))
}
pub(crate) fn valid(
    program: &Path,
    args: &[String],
    env: &std::collections::BTreeMap<String, String>,
) -> bool {
    program.is_absolute()
        && program.file_name().and_then(|v| v.to_str()) == Some("node")
        && contract(args).is_some_and(|(role, _)| {
            env.get("HOME")
                .is_some_and(|home| crate::node_profile::environment(role, env, home))
        })
}
pub(crate) fn capture(args: &[String]) -> Result<(File, Vec<String>), &'static str> {
    capture_observed(args, |_| {})
}
fn capture_observed(
    args: &[String],
    mut observed: impl FnMut(u64),
) -> Result<(File, Vec<String>), &'static str> {
    let (_, lock) = contract(args).ok_or("Node entry contract")?;
    let path = Path::new(&lock.canonical_path);
    if !path.is_absolute() || path.canonicalize().map_err(|_| "Node entry path")? != path {
        return Err("Node entry alias");
    }
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed())
        .open(path)
        .map_err(|_| "Node entry open")?;
    let before = source.metadata().map_err(|_| "Node entry metadata")?;
    if !before.is_file()
        || before.len() < 64
        || before.len() > 2 * 1024 * 1024
        || identity(&before)? != lock.file_identity
    {
        return Err("Node entry identity");
    }
    let mut target = File::from(
        rustix::fs::memfd_create(
            c"gent-node-entry",
            rustix::fs::MemfdFlags::CLOEXEC
                | rustix::fs::MemfdFlags::ALLOW_SEALING
                | rustix::fs::MemfdFlags::NOEXEC_SEAL,
        )
        .map_err(|_| "Node nonexecutable snapshot")?,
    );
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 65536];
    let mut count = 0u64;
    loop {
        let n = source.read(&mut buffer).map_err(|_| "Node entry read")?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > 2 * 1024 * 1024 {
            return Err("Node entry quota");
        }
        target
            .write_all(&buffer[..n])
            .map_err(|_| "Node entry snapshot")?;
        digest.update(&buffer[..n]);
        observed(count);
    }
    if count != before.len()
        || format!("{:x}", digest.finalize()) != lock.digest_sha256
        || stamp(&before)? != stamp(&source.metadata().map_err(|_| "Node entry handle changed")?)?
        || stamp(&before)?
            != stamp(&std::fs::metadata(path).map_err(|_| "Node entry path changed")?)?
    {
        return Err("Node entry changed");
    }
    rustix::fs::fcntl_add_seals(
        &target,
        rustix::fs::SealFlags::WRITE
            | rustix::fs::SealFlags::GROW
            | rustix::fs::SealFlags::SHRINK
            | rustix::fs::SealFlags::SEAL,
    )
    .map_err(|_| "Node entry seals")?;
    let entry_arguments =
        crate::node_profile::arguments(target.as_raw_fd()).ok_or("Node entry descriptor")?;
    Ok((target, entry_arguments))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> (tempfile::TempDir, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("entry.js");
        let data = b"export const qa = true;\n".repeat(8000);
        std::fs::write(&path, &data).unwrap();
        let meta = path.metadata().unwrap();
        let lock = RunVersionLock {
            provider: "node-mcp-entry".into(),
            canonical_path: path.to_str().unwrap().into(),
            file_identity: identity(&meta).unwrap(),
            digest_sha256: format!("{:x}", Sha256::digest(&data)),
            version: "qa".into(),
            compatibility_entry: "gent-node-bundle:gent-browser:v1".into(),
        };
        (
            root,
            vec!["gent-browser".into(), serde_json::to_string(&lock).unwrap()],
        )
    }
    #[test]
    fn snapshot_is_sealed_nonexecutable_data() {
        let (_root, args) = entry();
        let (mut snapshot, entry_arguments) = capture(&args).unwrap();
        let seals = rustix::fs::fcntl_get_seals(&snapshot).unwrap();
        assert!(seals.contains(
            rustix::fs::SealFlags::WRITE
                | rustix::fs::SealFlags::GROW
                | rustix::fs::SealFlags::SHRINK
                | rustix::fs::SealFlags::SEAL
        ));
        assert_eq!(snapshot.metadata().unwrap().mode() & 0o111, 0);
        assert!(snapshot.write_all(b"x").is_err());
        assert_eq!(
            std::process::Command::new(format!("/proc/self/fd/{}", snapshot.as_raw_fd()))
                .spawn()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(entry_arguments.len(), 3);
    }
    #[test]
    fn path_replacement_during_capture_is_rejected() {
        let (root, args) = entry();
        let mut replaced = false;
        assert!(
            capture_observed(&args, |_| {
                if !replaced {
                    let old = root.path().join("entry.js");
                    std::fs::rename(&old, root.path().join("old.js")).unwrap();
                    std::fs::write(old, b"replacement").unwrap();
                    replaced = true;
                }
            })
            .is_err()
        );
        assert!(replaced);
    }
    #[test]
    fn same_file_mutation_during_capture_is_rejected() {
        let (root, args) = entry();
        let mut changed = false;
        assert!(
            capture_observed(&args, |_| {
                if !changed {
                    std::fs::write(root.path().join("entry.js"), b"mutated").unwrap();
                    changed = true;
                }
            })
            .is_err()
        );
        assert!(changed);
    }
    #[test]
    fn directory_alias_wrong_hash_and_oversize_rejected() {
        for kind in ["directory", "alias", "hash", "oversize"] {
            let (root, mut args) = entry();
            let mut lock: RunVersionLock = serde_json::from_str(&args[1]).unwrap();
            match kind {
                "directory" => lock.canonical_path = root.path().to_str().unwrap().into(),
                "alias" => {
                    let alias = root.path().join("alias.js");
                    std::os::unix::fs::symlink(&lock.canonical_path, &alias).unwrap();
                    lock.canonical_path = alias.to_str().unwrap().into()
                }
                "hash" => lock.digest_sha256 = "0".repeat(64),
                _ => {
                    std::fs::write(&lock.canonical_path, vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
                    lock.file_identity =
                        identity(&std::fs::metadata(&lock.canonical_path).unwrap()).unwrap()
                }
            }
            args[1] = serde_json::to_string(&lock).unwrap();
            assert!(capture(&args).is_err(), "{kind}");
        }
    }
}
