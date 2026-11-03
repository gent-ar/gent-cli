//! Root-managed immutable runtime closure. No ambient executable or library paths.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePin {
    pub root: String,
    pub manifest_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    sha256: String,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn immutable(path: &Path, directory: bool) -> Result<(), &'static str> {
    let m = path.symlink_metadata().map_err(|_| "runtime metadata")?;
    if m.uid() != 0
        || m.mode() & 0o022 != 0
        || m.file_type().is_symlink()
        || m.is_dir() != directory
        || (!directory && !m.is_file())
    {
        return Err("runtime is not immutable");
    }
    Ok(())
}
fn ancestors(root: &Path) -> Result<(), &'static str> {
    for parent in root.ancestors().skip(1) {
        let m = parent.symlink_metadata().map_err(|_| "runtime ancestor")?;
        if !m.is_dir()
            || m.file_type().is_symlink()
            || m.uid() != 0
            || m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0
        {
            return Err("runtime ancestor mutable");
        }
    }
    Ok(())
}
impl RuntimePin {
    pub fn capture(root: &Path) -> Result<Self, &'static str> {
        let text = root.to_str().ok_or("runtime path")?;
        if !root.is_absolute() || root.canonicalize().map_err(|_| "runtime path")? != root {
            return Err("runtime path");
        }
        immutable(root, true)?;
        let file = root.join("manifest.json");
        immutable(&file, false)?;
        let bytes = std::fs::read(file).map_err(|_| "runtime manifest")?;
        if bytes.len() > 131_072 {
            return Err("runtime manifest bound");
        }
        let pin = Self {
            root: text.into(),
            manifest_sha256: hash(&bytes),
        };
        pin.verify()?;
        Ok(pin)
    }
    pub fn verify(&self) -> Result<(), &'static str> {
        let root = Path::new(&self.root);
        if !root.is_absolute() || root.canonicalize().map_err(|_| "runtime path")? != root {
            return Err("runtime path");
        }
        immutable(root, true)?;
        ancestors(root)?;
        let bytes = std::fs::read(root.join("manifest.json")).map_err(|_| "runtime manifest")?;
        if bytes.len() > 131_072 || hash(&bytes) != self.manifest_sha256 {
            return Err("runtime manifest changed");
        }
        immutable(&root.join("manifest.json"), false)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| "runtime manifest invalid")?;
        if manifest.schema != 1 || manifest.files.is_empty() || manifest.files.len() > 512 {
            return Err("runtime manifest count");
        }
        let mut names = std::collections::BTreeSet::new();
        let mut total = 0u64;
        for entry in manifest.files {
            let path = PathBuf::from(&entry.path);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                || !names.insert(entry.path)
            {
                return Err("runtime manifest path");
            }
            let full = root.join(&path);
            for parent in full.ancestors().skip(1) {
                if parent == root {
                    break;
                }
                immutable(parent, true)?;
            }
            immutable(&full, false)?;
            let size = full.metadata().map_err(|_| "runtime metadata")?.len();
            total = total.checked_add(size).ok_or("runtime size")?;
            if size > 16 * 1024 * 1024 || total > 64 * 1024 * 1024 {
                return Err("runtime size");
            }
            if hash(&std::fs::read(full).map_err(|_| "runtime read")?) != entry.sha256 {
                return Err("runtime digest changed");
            }
        }
        let mut observed = std::collections::BTreeSet::new();
        let mut pending = vec![root.to_owned()];
        let mut count = 0;
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).map_err(|_| "runtime tree")? {
                let entry = entry.map_err(|_| "runtime tree")?;
                let path = entry.path();
                let metadata = path.symlink_metadata().map_err(|_| "runtime tree")?;
                count += 1;
                if count > 1024 {
                    return Err("runtime tree bound");
                }
                if metadata.is_dir() {
                    immutable(&path, true)?;
                    pending.push(path);
                } else {
                    immutable(&path, false)?;
                    let relative = path
                        .strip_prefix(root)
                        .map_err(|_| "runtime tree")?
                        .to_str()
                        .ok_or("runtime path encoding")?
                        .to_owned();
                    if relative != "manifest.json" {
                        observed.insert(relative);
                    }
                }
            }
        }
        if observed != names {
            return Err("unmanifested runtime file");
        }
        for required in [
            "usr/bin/slirp4netns",
            "usr/sbin/iptables",
            "usr/sbin/ip6tables",
            "usr/sbin/iptables-restore",
            "usr/sbin/ip6tables-restore",
        ] {
            if !names.contains(required) {
                return Err("runtime executable missing");
            }
        }
        Ok(())
    }
    /// Public trusted runtime paths for the owned helper process only. The
    /// caller verifies this pin before process creation; no ambient inheritance.
    pub fn helper_environment(&self) -> Result<Vec<(String, String)>, &'static str> {
        self.verify()?;
        Ok(vec![
            (
                "PATH".into(),
                format!("{0}/usr/sbin:{0}/usr/bin:/usr/bin:/bin", self.root),
            ),
            (
                "LD_LIBRARY_PATH".into(),
                format!("{}/usr/lib/x86_64-linux-gnu", self.root),
            ),
            (
                "XTABLES_LIBDIR".into(),
                format!("{}/usr/lib/x86_64-linux-gnu/xtables", self.root),
            ),
        ])
    }
    pub fn verify_helper_environment(&self) -> Result<(), &'static str> {
        for (key, value) in self.helper_environment()? {
            if std::env::var(&key).ok().as_deref() != Some(value.as_str()) {
                return Err("runtime helper environment changed");
            }
        }
        Ok(())
    }
}
