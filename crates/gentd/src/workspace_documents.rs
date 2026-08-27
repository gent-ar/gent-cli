use gent_protocol::{WorkspaceDocumentGroup, WorkspaceDocumentRecord};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_FILES: usize = 256;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub fn discover(root: &Path) -> Result<Vec<WorkspaceDocumentRecord>, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    for (name, group) in [
        ("Project", WorkspaceDocumentGroup::Project),
        (".gent", WorkspaceDocumentGroup::Gent),
        ("Docs", WorkspaceDocumentGroup::Docs),
    ] {
        let dir = root.join(name);
        if dir.is_dir() {
            walk(&root, &dir, group, &mut records)?;
        }
    }
    records.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(records)
}

fn walk(
    root: &Path,
    dir: &Path,
    group: WorkspaceDocumentGroup,
    out: &mut Vec<WorkspaceDocumentRecord>,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            walk(root, &path, group.clone(), out)?;
            continue;
        }
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        if out.len() >= MAX_FILES {
            return Ok(());
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let absolute = path.to_string_lossy().into_owned();
        let mut digest = Sha256::new();
        digest.update(relative.as_bytes());
        let document_id = format!("doc-{}", hex::encode(digest.finalize()));
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_secs());
        out.push(WorkspaceDocumentRecord {
            document_id,
            group: group.clone(),
            relative_path: relative,
            absolute_path: absolute,
            byte_len: metadata.len(),
            modified_unix_seconds: modified,
        });
    }
    Ok(())
}
