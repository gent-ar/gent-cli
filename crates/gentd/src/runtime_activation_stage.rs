use std::{
    fmt::Write as _,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::files;

const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct StatEntry {
    name: String,
    bytes: u64,
    modified_nanos: u128,
    permissions: u32,
}

struct Walked {
    name: String,
    path: PathBuf,
    metadata: fs::Metadata,
    permissions: u32,
}

pub(super) struct Tree {
    pub(super) fingerprint: String,
    directories: Vec<String>,
    files: Vec<(String, PathBuf, u32, [u8; 32])>,
}

impl Tree {
    pub(super) fn digest(&self, name: &str) -> Result<String, String> {
        self.files
            .iter()
            .find(|(candidate, ..)| candidate == name)
            .map(|(.., hash)| hex(hash))
            .ok_or_else(|| format!("runtime tree has no {name}"))
    }
}

pub(super) fn publish(tree: &Tree, root: &Path, destination: &Path) -> Result<(), String> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err("immutable runtime generation already exists".into());
    }
    let temporary = tempfile::Builder::new()
        .prefix(".stage-")
        .tempdir_in(root)
        .map_err(super::display)?;
    for directory in &tree.directories {
        fs::create_dir_all(temporary.path().join(directory)).map_err(super::display)?;
    }
    for (name, source, _, expected) in &tree.files {
        let target = temporary.path().join(name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(super::display)?;
        }
        if copy_hashed(source, &target)? != *expected {
            return Err(
                "runtime bootstrap changed while preparing its immutable generation".into(),
            );
        }
    }
    fs::rename(temporary.path(), destination).map_err(super::display)
}

pub(super) fn scan(root: &Path) -> Result<Tree, String> {
    let (directories, walked) = walk(root)?;
    let mut tree = Sha256::new();
    tree.update(b"gent-runtime-tree-v1\0");
    let mut files = Vec::with_capacity(walked.len());
    for entry in walked {
        let hash = file_hash(&entry.path)?;
        tree.update((entry.name.len() as u64).to_be_bytes());
        tree.update(entry.name.as_bytes());
        tree.update(hash);
        tree.update(entry.permissions.to_be_bytes());
        files.push((entry.name, entry.path, entry.permissions, hash));
    }
    Ok(Tree {
        fingerprint: hex(&tree.finalize()),
        directories,
        files,
    })
}

pub(super) fn fingerprint(root: &Path) -> Result<String, String> {
    scan(root).map(|tree| tree.fingerprint)
}

pub(super) fn stat(root: &Path) -> Result<Vec<StatEntry>, String> {
    let (_, walked) = walk(root)?;
    walked
        .into_iter()
        .map(|entry| {
            let modified = entry
                .metadata
                .modified()
                .map_err(super::display)?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(super::display)?;
            Ok(StatEntry {
                name: entry.name,
                bytes: entry.metadata.len(),
                modified_nanos: modified.as_nanos(),
                permissions: entry.permissions,
            })
        })
        .collect()
}

pub(super) fn file_digest(path: &Path) -> Result<String, String> {
    file_hash(path).map(|hash| hex(&hash))
}

fn walk(root: &Path) -> Result<(Vec<String>, Vec<Walked>), String> {
    let metadata = fs::symlink_metadata(root).map_err(super::display)?;
    if !metadata.is_dir() || files::redirected(&metadata) {
        return Err("runtime source is not a plain directory".into());
    }
    let mut directories = Vec::new();
    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut visited = 0;
    collect(
        root,
        root,
        &mut directories,
        &mut entries,
        (&mut bytes, &mut visited),
        0,
    )?;
    directories.sort();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok((directories, entries))
}

fn relative(root: &Path, path: &Path) -> Result<String, String> {
    Ok(path
        .strip_prefix(root)
        .map_err(super::display)?
        .to_str()
        .ok_or("runtime source name is not UTF-8")?
        .replace('\\', "/"))
}

fn collect(
    root: &Path,
    directory: &Path,
    directories: &mut Vec<String>,
    entries: &mut Vec<Walked>,
    (bytes, visited): (&mut u64, &mut usize),
    depth: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("runtime source directory depth exceeds its verification bound".into());
    }
    for entry in fs::read_dir(directory).map_err(super::display)? {
        *visited += 1;
        if *visited > 40_000 {
            return Err("runtime source entry count exceeds its verification bound".into());
        }
        let path = entry.map_err(super::display)?.path();
        let metadata = fs::symlink_metadata(&path).map_err(super::display)?;
        if files::redirected(&metadata) {
            return Err("runtime source contains a redirected entry".into());
        }
        if metadata.is_dir() {
            directories.push(relative(root, &path)?);
            collect(
                root,
                &path,
                directories,
                entries,
                (&mut *bytes, &mut *visited),
                depth + 1,
            )?;
        } else if metadata.is_file() {
            *bytes = bytes
                .checked_add(metadata.len())
                .ok_or("runtime source size overflow")?;
            if entries.len() >= MAX_FILES || *bytes > MAX_BYTES {
                return Err("runtime source exceeds its verification bound".into());
            }
            entries.push(Walked {
                name: relative(root, &path)?,
                permissions: permissions(&metadata)?,
                path,
                metadata,
            });
        } else {
            return Err("runtime source contains an unsupported entry".into());
        }
    }
    Ok(())
}

fn permissions(metadata: &fs::Metadata) -> Result<u32, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o7000 != 0 {
            return Err("runtime source has unsupported special permissions".into());
        }
        Ok(mode & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Ok(0)
    }
}

fn copy_hashed(source: &Path, target: &Path) -> Result<[u8; 32], String> {
    let metadata = fs::symlink_metadata(source).map_err(super::display)?;
    if !metadata.is_file() || files::redirected(&metadata) {
        return Err("runtime file is missing or redirected".into());
    }
    let input = fs::File::open(source).map_err(super::display)?;
    let mut output = fs::File::create_new(target).map_err(super::display)?;
    let hash = stream(&input, Some(&mut output))?;
    output.sync_all().map_err(super::display)?;
    fs::set_permissions(target, metadata.permissions()).map_err(super::display)?;
    Ok(hash)
}

fn file_hash(path: &Path) -> Result<[u8; 32], String> {
    let metadata = fs::symlink_metadata(path).map_err(super::display)?;
    if !metadata.is_file() || files::redirected(&metadata) {
        return Err("runtime file is missing or redirected".into());
    }
    let file = fs::File::open(path).map_err(super::display)?;
    stream(&file, None)
}

pub(super) fn file_digest_handle(file: &fs::File) -> Result<String, String> {
    stream(file, None).map(|hash| hex(&hash))
}

fn stream(mut file: &fs::File, mut output: Option<&mut fs::File>) -> Result<[u8; 32], String> {
    #[cfg(test)]
    tests::HASHED_FILES.with(|count| count.set(count.get() + 1));
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut bytes: u64 = 0;
    loop {
        let read = file.read(&mut buffer).map_err(super::display)?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or("runtime file size overflow")?;
        if bytes > MAX_BYTES {
            return Err("runtime file exceeds its verification bound".into());
        }
        hash.update(&buffer[..read]);
        if let Some(output) = output.as_mut() {
            output.write_all(&buffer[..read]).map_err(super::display)?;
        }
    }
    Ok(hash.finalize().into())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[cfg(test)]
pub(super) mod tests {
    std::thread_local! {
        pub(crate) static HASHED_FILES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    pub(crate) fn hashed_files() -> usize {
        HASHED_FILES.with(std::cell::Cell::get)
    }
}
